//! Opens a bot's memory session from a host's [`FlowLikeState`]: the workspace's LanceDB
//! connection (tables appear in the Data tab), the bot's embedding model, and the turn's model.
//! One session per bot and conversation lives in a process-wide registry; each turn hands it
//! fresh credentials.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

use flow_like_bot_memory::compaction::CompactionConfig;
use flow_like_bot_memory::host::{
    MemorySession, ModelEmbedder, ModelLlm, SessionOptions, TurnRequest,
};
use flow_like_bot_memory::{Embedder, MemoryLlm, ModelBudget};
use flow_like_storage::object_store::path::Path;
use flow_like_types::Result;

use super::bot_run::BotRunContext;
use super::memory_tools::{HeadlessMemory, MemoryTurnSource};
use crate::bit::Bit;
use crate::models::llm::ModelUsageContext;
use crate::profile::Profile;
use crate::state::FlowLikeState;

const SESSION_IDLE: Duration = Duration::from_secs(30 * 60);
/// A session older than this is opened again: the workspace connection's credentials expire.
const SESSION_MAX_AGE: Duration = Duration::from_secs(10 * 60);
const DEFAULT_CONTEXT: u32 = 32_768;
const MAX_CONTEXT: u32 = 200_000;
const OUTPUT_TOKENS: u32 = 4_096;

struct Cached {
    session: Arc<MemorySession>,
    used: Instant,
    opened: Instant,
}

static SESSIONS: LazyLock<Mutex<HashMap<String, Cached>>> = LazyLock::new(Default::default);

fn cached(key: &str) -> Option<Arc<MemorySession>> {
    let mut sessions = SESSIONS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    sessions.retain(|_, entry| {
        entry.used.elapsed() < SESSION_IDLE && entry.opened.elapsed() < SESSION_MAX_AGE
    });
    let entry = sessions.get_mut(key)?;
    entry.used = Instant::now();
    Some(entry.session.clone())
}

fn remember(key: String, session: Arc<MemorySession>) {
    SESSIONS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .insert(
            key,
            Cached {
                session,
                used: Instant::now(),
                opened: Instant::now(),
            },
        );
}

fn evict(key: &str) {
    SESSIONS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .remove(key);
}

/// Forgets a bot's sessions, e.g. after its workspace or memory was reset.
pub fn forget_bot_memory(bot_id: &str) {
    let prefix = format!("{bot_id}/");
    SESSIONS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .retain(|key, _| !key.starts_with(&prefix));
}

/// The prompt budget and compaction config of the model that answers.
pub fn budget_for(model: &Bit) -> (ModelBudget, CompactionConfig) {
    let context = model
        .try_to_llm()
        .map(|parameters| parameters.context_length)
        .or_else(|| {
            model
                .try_to_vlm()
                .map(|parameters| parameters.context_length)
        })
        .filter(|context| *context > 0)
        .unwrap_or(DEFAULT_CONTEXT);
    match context {
        ..=4_096 => (ModelBudget::mlx_4k(), CompactionConfig::small_model()),
        ..=8_192 => (ModelBudget::llama_cpp_8k(), CompactionConfig::small_model()),
        context => (
            ModelBudget::hosted(context.min(MAX_CONTEXT), OUTPUT_TOKENS),
            CompactionConfig::default(),
        ),
    }
}

/// The LanceDB connection of an app's project database, where the app's tables live.
pub async fn workspace_connection(
    state: &Arc<FlowLikeState>,
    app_id: &str,
    provided: Option<flow_like_storage::lancedb::Connection>,
) -> Result<(
    flow_like_storage::lancedb::Connection,
    Option<flow_like_storage::lancedb::table::WriteOptions>,
)> {
    let (build, write_options) = {
        let config = state.config.read().await;
        (
            config.callbacks.build_project_database.clone(),
            config.callbacks.lance_write_options.clone(),
        )
    };
    if let Some(connection) = provided {
        return Ok((connection, write_options));
    }
    let build =
        build.ok_or_else(|| flow_like_types::anyhow!("No project database builder registered"))?;
    let path = Path::from("apps").join(app_id).join("storage").join("db");
    let connection = state
        .with_lance_session(build(path.clone()))
        .execute()
        .await
        .map_err(|error| {
            flow_like_types::anyhow!("Opening the workspace database {path} failed: {error}")
        })?;
    Ok((connection, write_options))
}

/// What a host hands over besides the conversation.
pub struct MemoryModels<'a> {
    pub state: &'a Arc<FlowLikeState>,
    pub profile: &'a Profile,
    /// The model that answers the turn; it also writes summaries and extracts facts.
    pub completion: &'a Bit,
    /// The chat's embedding model, used when the bot has none of its own.
    pub chat_embedding: Option<&'a str>,
    pub token: Option<String>,
    pub run_id: Option<&'a str>,
    /// The workspace's database when the host opens it itself (scoped credentials on a server);
    /// otherwise the state's project database builder opens it.
    pub connection: Option<flow_like_storage::lancedb::Connection>,
}

async fn build_models(
    models: &MemoryModels<'_>,
    embedding_id: &str,
) -> Result<(Arc<dyn MemoryLlm>, Arc<dyn Embedder>)> {
    let usage = ModelUsageContext {
        app_id: None,
        run_id: models.run_id.map(str::to_string),
        api_base_url: None,
    };
    let bit = models
        .profile
        .find_bit(embedding_id, models.state.http_client.clone())
        .await?;
    let logic = models
        .state
        .embedding_factory
        .build_text_routed(
            &bit,
            models.state.clone(),
            models.token.clone(),
            Some(usage.clone()),
        )
        .await?;
    let model = models
        .state
        .model_factory
        .build(
            models.completion,
            models.state.clone(),
            models.token.clone(),
            Some(usage),
        )
        .await?;
    let name = model
        .default_model()
        .await
        .unwrap_or_else(|| models.completion.id.clone());
    Ok((
        Arc::new(ModelLlm::new(model, name)),
        Arc::new(ModelEmbedder::new(embedding_id, logic)),
    ))
}

/// Opens the bot's memory for this turn and prepares the prompt. `None` when the bot does not
/// remember, has no embedding model or workspace, or the memory cannot be opened: the host then
/// answers the way it did before the memory engine. Never fails the turn.
pub async fn open_headless_memory(
    models: &MemoryModels<'_>,
    bot: &BotRunContext,
    source: MemoryTurnSource,
) -> Option<HeadlessMemory> {
    let embedding_id = bot.memory_model(models.chat_embedding)?.to_string();
    let app_id = bot.workspace_app_id.as_deref()?;
    let key = format!("{}/{}/{embedding_id}", bot.bot_id, source.key);
    let session = match open_session(models, &key, &embedding_id, app_id, &source).await {
        Ok(session) => session,
        Err(error) => {
            tracing::warn!(bot_id = %bot.bot_id, "Bot memory is unavailable this turn: {error:#}");
            return None;
        }
    };
    let request = TurnRequest {
        trigger_seq: source.trigger_seq,
        state_block: &source.state_block,
        about_you: "",
        open_task_seqs: &source.open_task_seqs,
        now: chrono::Utc::now(),
    };
    match session.prepare(source.reader.as_ref(), &request).await {
        Ok(prepared) => Some(HeadlessMemory::new(session, prepared, source)),
        Err(error) => {
            evict(&key);
            tracing::warn!(bot_id = %bot.bot_id, "Bot memory could not prepare the turn: {error:#}");
            None
        }
    }
}

async fn open_session(
    models: &MemoryModels<'_>,
    key: &str,
    embedding_id: &str,
    app_id: &str,
    source: &MemoryTurnSource,
) -> Result<Arc<MemorySession>> {
    let (llm, embedder) = build_models(models, embedding_id).await?;
    if let Some(session) = cached(key)
        && session.refresh(llm.clone(), embedder.clone())
    {
        return Ok(session);
    }
    let (opened, write_options) =
        workspace_connection(models.state, app_id, models.connection.clone()).await?;
    let connection = opened;
    let (budget, config) = budget_for(models.completion);
    let mut options = SessionOptions::new(budget, source.timezone);
    options.config = config;
    options.model = Some(models.completion.id.clone());
    let session = Arc::new(
        MemorySession::open(
            connection,
            write_options,
            llm,
            embedder,
            source.sink.clone(),
            options,
        )
        .await,
    );
    remember(key.to_string(), session.clone());
    Ok(session)
}

/// Opens the memory of a bot's run on a host that resolved the bot before it picked the model:
/// resolves `model_id` (or the profile's best model) as the one that answers, opens the memory
/// and attaches the memory tools to `bot`. `None` leaves both untouched.
#[allow(clippy::too_many_arguments)]
pub async fn open_for_run(
    state: &Arc<FlowLikeState>,
    profile: &Profile,
    bot: &mut Arc<BotRunContext>,
    model_id: Option<&str>,
    source: MemoryTurnSource,
    chat_embedding: Option<&str>,
    token: Option<String>,
    run_id: Option<&str>,
) -> Option<HeadlessMemory> {
    if !bot.remembers() {
        return None;
    }
    let completion = match super::bot_run::resolve_host_completion_model(state, profile, model_id)
        .await
    {
        Ok(completion) => completion,
        Err(error) => {
            tracing::warn!(bot_id = %bot.bot_id, "Bot memory found no model to write summaries with: {error:#}");
            return None;
        }
    };
    let models = MemoryModels {
        state,
        profile,
        completion: &completion,
        chat_embedding,
        token,
        run_id,
        connection: None,
    };
    let memory = open_headless_memory(&models, bot.as_ref(), source).await?;
    let context = Arc::make_mut(bot);
    context.host_tools = memory.host_tools(std::mem::take(&mut context.host_tools));
    Some(memory)
}

/// The reply of a turn is stored: runs the background follow-up (Soft compaction, fact mining,
/// deletions, publishing) on the bot's open session for this conversation, if it has one.
pub fn follow_up(bot_id: &str, source: MemoryTurnSource) {
    let prefix = format!("{bot_id}/{}/", source.key);
    let sessions: Vec<Arc<MemorySession>> = SESSIONS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .iter()
        .filter(|(key, _)| key.starts_with(&prefix))
        .map(|(_, entry)| entry.session.clone())
        .collect();
    for session in sessions {
        let source = source.clone();
        flow_like_types::tokio::spawn(async move {
            let request = TurnRequest {
                trigger_seq: source.trigger_seq,
                state_block: &source.state_block,
                about_you: "",
                open_task_seqs: &source.open_task_seqs,
                now: chrono::Utc::now(),
            };
            let done = session.after_turn(source.reader.as_ref(), &request).await;
            tracing::debug!(?done, key = %source.key, "Bot memory follow-up finished");
        });
    }
}

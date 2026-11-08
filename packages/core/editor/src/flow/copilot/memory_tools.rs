//! Bot memory on headless turns: the memory engine (`flow-like-bot-memory`) opened on the bot's
//! workspace, the prompt it assembles instead of "the last entries", and the memory tools the
//! turn offers. Hosts supply the inbox ([`MemoryTurnSource`]); everything else is resolved from the
//! host's own state. Interactive inbox turns use the same source and tools.

use std::fmt;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use async_trait::async_trait;
use flow_like_bot_memory::host::{
    MemorySession, MemoryToolbox, PreparedTurn, SegmentSink, TurnRequest,
};
use flow_like_bot_memory::model::{Role, Seq};
use flow_like_bot_memory::tools::{HistoryReader, tool_definitions};
use flow_like_types::Value;
use serde_json::json;

use super::platform::PlatformToolBridge;
use super::tool_spec::{PlatformToolSpec, ToolApprovalSpec};
use super::types::{ChatMessage, ChatRole};

const TOOL_TIMEOUT_SECS: u64 = 60;

/// The memory tools a host runs for one turn, as [`super::HostTools`] carries them.
#[derive(Clone)]
pub struct MemoryHandle(pub(crate) Arc<MemoryToolbox>);

impl MemoryHandle {
    pub fn new(toolbox: Arc<MemoryToolbox>) -> Self {
        Self(toolbox)
    }

    pub fn toolbox(&self) -> &Arc<MemoryToolbox> {
        &self.0
    }
}

impl fmt::Debug for MemoryHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("MemoryHandle")
    }
}

impl PartialEq for MemoryHandle {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for MemoryHandle {}

macro_rules! memory_schemas {
    ($($name:literal),+ $(,)?) => {
        fn schema_fn(name: &str) -> Option<fn() -> Value> {
            match name {
                $($name => Some((|| schema_of($name)) as fn() -> Value),)+
                _ => None,
            }
        }
    };
}

fn schema_of(name: &str) -> Value {
    tool_definitions()
        .into_iter()
        .find(|definition| definition.name == name)
        .map_or(json!({ "type": "object" }), |definition| {
            definition.parameters
        })
}

memory_schemas!(
    "memory_search",
    "history_search",
    "history_read",
    "message_get",
    "summary_expand",
    "memory_save",
    "memory_forget",
);

/// The specs of the memory tools. Writes need no approval round trip: the toolbox itself holds a
/// write back while web or tool content is part of the turn.
pub fn memory_tool_specs() -> Vec<PlatformToolSpec> {
    tool_definitions()
        .into_iter()
        .filter_map(|definition| {
            Some(PlatformToolSpec {
                name: definition.name,
                description: definition.description,
                schema: schema_fn(definition.name)?,
                approval: ToolApprovalSpec::None,
                timeout_secs: TOOL_TIMEOUT_SECS,
            })
        })
        .collect()
}

/// Answers the memory tools from the turn's toolbox and hands every other call on.
pub(super) struct MemoryBridge {
    pub(super) toolbox: Arc<MemoryToolbox>,
    pub(super) inner: Arc<dyn PlatformToolBridge>,
}

#[async_trait]
impl PlatformToolBridge for MemoryBridge {
    async fn call(&self, tool_name: &str, arguments: Value) -> String {
        if MemoryToolbox::handles(tool_name) {
            return self.toolbox.call(tool_name, arguments).await;
        }
        self.toolbox.note_foreign_call();
        self.inner.call(tool_name, arguments).await
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

/// What a host knows about the conversation a turn answers.
#[derive(Clone)]
pub struct MemoryTurnSource {
    /// The bot's copy of the conversation, with each speaker identified.
    pub reader: Arc<dyn HistoryReader>,
    /// Where the bot's summaries are written, as the bot.
    pub sink: Arc<dyn SegmentSink>,
    /// Names the bot, the conversation and the stores; a session is kept per key.
    pub key: String,
    /// Group history has its own positions and stores.
    pub room: bool,
    /// Whether this conversation may recall the bot owner's private memory.
    pub use_owner_memory: bool,
    /// Group stores that currently allow this bot to reuse their facts here.
    pub shared_room_keys: Vec<String>,
    /// The position of the message the turn answers.
    pub trigger_seq: Seq,
    pub timezone: flow_like_bot_memory::host::Tz,
    /// Open tasks, approvals and running jobs, rendered from the host's records.
    pub state_block: String,
    pub open_task_seqs: Vec<Seq>,
}

impl MemoryTurnSource {
    fn request(&self) -> TurnRequest<'_> {
        TurnRequest {
            trigger_seq: self.trigger_seq,
            state_block: &self.state_block,
            about_you: "",
            open_task_seqs: &self.open_task_seqs,
            now: chrono::Utc::now(),
        }
    }
}

/// A headless turn with memory: the prompt the engine assembled and the tools to offer.
pub struct HeadlessMemory {
    session: Arc<MemorySession>,
    toolbox: Arc<MemoryToolbox>,
    prepared: PreparedTurn,
    source: MemoryTurnSource,
}

impl HeadlessMemory {
    pub(crate) fn new(
        session: Arc<MemorySession>,
        prepared: PreparedTurn,
        source: MemoryTurnSource,
    ) -> Self {
        let toolbox = Arc::new(MemoryToolbox::new(
            session.clone(),
            source.reader.clone(),
            prepared.trigger.clone(),
        ));
        Self {
            session,
            toolbox,
            prepared,
            source,
        }
    }

    /// Replaces the toolbox's write flag, so a failed attempt that saved a memory is not
    /// repeated.
    pub fn with_write_flag(mut self, wrote: Arc<AtomicBool>) -> Self {
        let toolbox = MemoryToolbox::new(
            self.session.clone(),
            self.source.reader.clone(),
            self.prepared.trigger.clone(),
        )
        .with_write_flag(wrote);
        self.toolbox = Arc::new(toolbox);
        self
    }

    /// About-you, summaries and state, for the end of the system prompt.
    pub fn context(&self) -> &str {
        &self.prepared.context
    }

    /// The recalled facts and the message the turn answers.
    pub fn prompt(&self) -> &str {
        &self.prepared.prompt
    }

    pub fn prepared(&self) -> &PreparedTurn {
        &self.prepared
    }

    pub fn session(&self) -> &Arc<MemorySession> {
        &self.session
    }

    /// The verbatim tail as chat history, oldest first.
    pub fn history(&self) -> Vec<ChatMessage> {
        self.prepared
            .history
            .iter()
            .map(|message| ChatMessage {
                role: if message.role == Role::Assistant {
                    ChatRole::Assistant
                } else {
                    ChatRole::User
                },
                content: message.text.clone(),
                images: None,
            })
            .collect()
    }

    /// `base` with this turn's memory tools attached.
    pub fn host_tools(&self, base: super::HostTools) -> super::HostTools {
        base.with_memory(MemoryHandle::new(self.toolbox.clone()))
    }

    pub fn toolbox(&self) -> &Arc<MemoryToolbox> {
        &self.toolbox
    }

    /// The reply is stored: the Soft compaction, fact mining, deletions and summary publishing
    /// run in the background, under the engine's lease.
    pub fn finish(self) {
        let (session, source) = (self.session, self.source);
        flow_like_types::tokio::spawn(async move {
            let request = source.request();
            let done = session.after_turn(source.reader.as_ref(), &request).await;
            tracing::debug!(?done, key = %source.key, "Bot memory follow-up finished");
        });
    }
}

#[cfg(test)]
pub(crate) async fn test_handle() -> MemoryHandle {
    use flow_like_bot_memory::host::{
        PublishedSegment, SessionOptions, SwitchEmbedder, SwitchLlm, Tz,
    };
    use flow_like_bot_memory::store::MemoryStores;
    use flow_like_bot_memory::testing::{HashEmbedder, MemoryHistory, StubLlm};
    use flow_like_bot_memory::{HistoryItem, ModelBudget};

    struct NoSink;

    #[async_trait]
    impl SegmentSink for NoSink {
        async fn published(&self) -> flow_like_types::Result<std::collections::HashSet<String>> {
            Ok(Default::default())
        }

        async fn write(&self, _segment: &PublishedSegment) -> flow_like_types::Result<()> {
            Ok(())
        }
    }

    let dir = std::env::temp_dir().join(format!("memory-tools-{}", flow_like_types::create_id()));
    let embedder = Arc::new(SwitchEmbedder::new(Arc::new(HashEmbedder::new("hash", 16))));
    let stores = MemoryStores::open_local(&dir, embedder.clone())
        .await
        .expect("memory stores");
    let session = Arc::new(MemorySession::new(
        stores,
        Arc::new(SwitchLlm::new(Arc::new(StubLlm::new()))),
        embedder,
        Arc::new(NoSink),
        SessionOptions::new(ModelBudget::hosted(8_192, 1_024), Tz::UTC),
    ));
    let trigger = HistoryItem::new(
        1,
        "m1",
        Role::User,
        chrono::Utc::now(),
        "Please remember that my cat is called Mia.",
    );
    let history = MemoryHistory::new();
    history.push(trigger.clone());
    MemoryHandle::new(Arc::new(MemoryToolbox::new(
        session,
        Arc::new(history),
        trigger,
    )))
}

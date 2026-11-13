//! One bot's memory on one host: the engine, its models and the inbox as the source of truth for
//! segments. A headless turn calls [`MemorySession::prepare`] (Hard compaction, then the prompt
//! from summaries, state block, verbatim tail and recalled facts) and, once the reply is stored,
//! [`MemorySession::after_turn`] in the background (Soft compaction, fact mining, deletions and
//! periodic expiry cleanup and index maintenance).
//! Memory never fails a turn on its own: every step that goes wrong is logged and skipped.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Instant;

use chrono::{DateTime, Utc};
use chrono_tz::Tz;
use flow_like_model_provider::response::{CompactionStats, ContextUsage, LLMUsageStats};
use flow_like_storage::lancedb::Connection;
use flow_like_storage::lancedb::table::WriteOptions;
use flow_like_types::tokio::sync::Mutex;
use flow_like_types::{Result, anyhow};

use super::models::{SwitchEmbedder, SwitchLlm};
use super::publish::{SegmentSink, publish_summaries};
use crate::compaction::{CompactionConfig, TurnPhase};
use crate::engine::{CompactionInput, DeletionReport, MemoryBusy, MemoryEngine};
use crate::llm::{Embedder, MemoryLlm};
use crate::model::{HistoryItem, Role, Seq};
use crate::prompt::{
    BudgetReport, ModelBudget, PromptInput, SectionKind, assemble, render_tail_item,
};
use crate::retrieval::Recency;
use crate::store::{FactFilter, FactQuery, MemoryStores};
use crate::telemetry::UsageDelivery;
use crate::tools::HistoryReader;

const DELETIONS_PER_SWEEP: usize = 200;
const MAX_COMPACTION_ATTEMPTS: usize = 2;

#[derive(Debug, Clone)]
pub struct SessionOptions {
    pub budget: ModelBudget,
    pub config: CompactionConfig,
    pub timezone: Tz,
    /// Tokens of the tool definitions sent next to the prompt.
    pub tool_tokens: u32,
    /// Messages after the last mined one that make a turn's background work mine facts.
    pub mine_after: u64,
    /// The model summaries are written with, recorded next to them.
    pub model: Option<String>,
    /// Groups keep their history and learned statements apart from the owner's direct chat.
    pub room_key: Option<String>,
    /// Whether this group may recall the owner's private facts.
    pub use_owner_memory: bool,
    /// Group sources whose saved facts the host permits this conversation to recall.
    pub shared_room_keys: Vec<String>,
}

impl SessionOptions {
    pub fn new(budget: ModelBudget, timezone: Tz) -> Self {
        Self {
            budget,
            config: CompactionConfig::default(),
            timezone,
            tool_tokens: 0,
            mine_after: 6,
            model: None,
            room_key: None,
            use_owner_memory: false,
            shared_room_keys: Vec::new(),
        }
    }
}

/// What the turn needs to know besides the history.
#[derive(Debug, Clone)]
pub struct TurnRequest<'a> {
    /// The message the turn answers.
    pub trigger_seq: Seq,
    /// Open tasks, approvals and running jobs, rendered from the host's records.
    pub state_block: &'a str,
    pub about_you: &'a str,
    /// The open task's user messages, which stay verbatim.
    pub open_task_seqs: &'a [Seq],
    pub now: DateTime<Utc>,
}

/// One message of the verbatim tail, as chat history.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnMessage {
    pub role: Role,
    pub text: String,
}

#[derive(Debug, Clone)]
pub struct PreparedTurn {
    /// About-you, summaries and state: goes after the system prompt, cache-stable.
    pub context: String,
    /// The verbatim tail before the trigger, oldest first.
    pub history: Vec<TurnMessage>,
    /// The recalled facts and the message the turn answers.
    pub prompt: String,
    /// The message the turn answers.
    pub trigger: HistoryItem,
    pub report: BudgetReport,
    /// A Hard compaction ran before the call.
    pub compacted: bool,
    /// This preparation and unreported maintenance from earlier turns.
    pub stats: LLMUsageStats,
    /// Transfers these measurements to the host once; an abandoned turn refunds them.
    pub usage_delivery: UsageDelivery,
}

/// What a turn's background work did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AfterTurn {
    pub compacted: bool,
    pub mined: bool,
    pub rewrote: bool,
    pub deletions: usize,
    pub published: usize,
    /// Periodic expiry cleanup, embedding and index maintenance completed.
    pub optimized: bool,
    pub expired_facts: usize,
    /// Another engine holds the memory's lease; the rest waits for the next turn.
    pub busy: bool,
}

pub struct MemorySession {
    engine: MemoryEngine,
    llm: Arc<SwitchLlm>,
    embedder: Arc<SwitchEmbedder>,
    sink: Arc<dyn SegmentSink>,
    options: SessionOptions,
    published: Mutex<Option<HashSet<String>>>,
}

fn is_busy(error: &flow_like_types::Error) -> bool {
    error.is::<MemoryBusy>() || error.is::<crate::store::StateConflict>()
}

impl MemorySession {
    pub fn new(
        stores: MemoryStores,
        llm: Arc<SwitchLlm>,
        embedder: Arc<SwitchEmbedder>,
        sink: Arc<dyn SegmentSink>,
        options: SessionOptions,
    ) -> Self {
        Self {
            engine: MemoryEngine::new(
                stores,
                llm.clone(),
                options.config.clone(),
                options.timezone,
            ),
            llm,
            embedder,
            sink,
            options,
            published: Mutex::new(None),
        }
    }

    /// The session on the bot workspace's LanceDB connection.
    pub async fn open(
        connection: Connection,
        write_options: Option<WriteOptions>,
        llm: Arc<dyn MemoryLlm>,
        embedder: Arc<dyn Embedder>,
        sink: Arc<dyn SegmentSink>,
        options: SessionOptions,
    ) -> Self {
        let llm = Arc::new(SwitchLlm::new(llm));
        let embedder = Arc::new(SwitchEmbedder::new(embedder));
        let stores = match options.room_key.as_deref() {
            Some(key) => {
                MemoryStores::open_room(connection, embedder.clone(), write_options, key).await
            }
            None => MemoryStores::open(connection, embedder.clone(), write_options).await,
        }
        .with_recall_sources(options.use_owner_memory, &options.shared_room_keys)
        .await;
        Self::new(stores, llm, embedder, sink, options)
    }

    pub fn engine(&self) -> &MemoryEngine {
        &self.engine
    }

    pub fn options(&self) -> &SessionOptions {
        &self.options
    }

    pub fn embedder_model(&self) -> &str {
        self.embedder.model_id()
    }

    /// Gives the next turn's models (fresh credentials) to the same engine. The embedder is
    /// swapped only for the same model; returns whether it was.
    pub fn refresh(&self, llm: Arc<dyn MemoryLlm>, embedder: Arc<dyn Embedder>) -> bool {
        self.llm.set(llm);
        self.embedder.set(embedder)
    }

    fn window(&self) -> u32 {
        self.options.budget.history_window()
    }

    /// Writes the summaries the inbox does not have yet; failures are logged, the next call
    /// tries again.
    pub async fn publish(&self) -> usize {
        let summaries = match self.engine.stores().segments.all().await {
            Ok(summaries) => summaries,
            Err(error) => {
                tracing::warn!("Reading the summaries to publish failed: {error:#}");
                return 0;
            }
        };
        let mut guard = self.published.lock().await;
        if guard.is_none() {
            match self.sink.published().await {
                Ok(known) => *guard = Some(known),
                Err(error) => {
                    tracing::warn!("Listing the published summaries failed: {error:#}");
                    return 0;
                }
            }
        }
        let known = guard.get_or_insert_with(HashSet::new);
        let (written, error) =
            publish_summaries(&summaries, self.options.timezone, self.sink.as_ref(), known).await;
        if let Some(error) = error {
            tracing::warn!("Publishing a summary to the inbox failed: {error:#}");
        }
        written
    }

    /// Follows deletions the history recorded since the last sweep. Returns how many messages.
    pub async fn sweep_deletions(
        &self,
        reader: &dyn HistoryReader,
        now: DateTime<Utc>,
    ) -> Result<(usize, DeletionReport)> {
        let state = self.engine.stores().state.load().await?;
        let from = state.deletions_through.unwrap_or(0);
        let deleted = reader.deleted_since(from, DELETIONS_PER_SWEEP).await?;
        if deleted.seqs.is_empty() && deleted.through <= from {
            return Ok((0, DeletionReport::default()));
        }
        let report = self
            .engine
            .follow_deletions(
                &deleted.seqs,
                Some(deleted.through),
                reader,
                self.window(),
                now,
            )
            .await?;
        self.publish().await;
        Ok((deleted.seqs.len(), report))
    }

    async fn compact(
        &self,
        reader: &dyn HistoryReader,
        trigger_seq: Seq,
        request: &TurnRequest<'_>,
        phase: TurnPhase,
    ) -> Result<bool> {
        for _ in 0..MAX_COMPACTION_ATTEMPTS {
            let state = self.engine.stores().state.load().await?;
            let items = self.engine.live_items(reader, &state, trigger_seq).await?;
            if let Err(error) = self.engine.index(&items).await {
                tracing::warn!("Indexing the history failed; history_search lags: {error:#}");
            }
            let plan = self
                .engine
                .plan(&items, &state, self.window(), request.open_task_seqs, phase)
                .await?;
            let Some(compaction) = plan.compaction else {
                return Ok(false);
            };
            let preceding = reader
                .before(
                    compaction.evict.from,
                    self.options.config.preceding_messages,
                )
                .await?;
            let started = Instant::now();
            let (outcome, stats) =
                crate::telemetry::capture(self.engine.compact(CompactionInput {
                    plan: &compaction,
                    items: &items,
                    preceding: &preceding,
                    state_block: request.state_block,
                    history_window: self.window(),
                    now: request.now,
                    model: self.options.model.clone(),
                }))
                .await;
            crate::telemetry::record(&stats);
            let mut after_tokens = None;
            if let Ok(outcome) = &outcome
                && !outcome.stale
                && let Ok(after) = self
                    .engine
                    .live_items(reader, &outcome.state, trigger_seq)
                    .await
                && let Ok(plan) = self
                    .engine
                    .plan(
                        &after,
                        &outcome.state,
                        self.window(),
                        request.open_task_seqs,
                        phase,
                    )
                    .await
            {
                after_tokens = Some(plan.used_tokens);
            }
            let event = CompactionStats {
                kind: "memory".into(),
                trigger: match compaction.trigger {
                    crate::compaction::Trigger::Hard => "hard",
                    crate::compaction::Trigger::Soft => "soft",
                    crate::compaction::Trigger::Idle => "idle",
                }
                .into(),
                status: match &outcome {
                    Ok(outcome) if outcome.stale => "stale",
                    Ok(_) => "completed",
                    Err(error) if is_busy(error) => "busy",
                    Err(_) => "failed",
                }
                .into(),
                before_tokens: compaction.tokens_before,
                after_tokens,
                duration_ms: started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64,
                model: stats
                    .calls
                    .iter()
                    .find(|call| call.purpose.as_deref() == Some("memory_segment_summary"))
                    .map(|call| call.model.clone())
                    .or_else(|| self.options.model.clone()),
                usage: (!stats.calls.is_empty()).then(|| stats.usage.clone()),
            };
            crate::telemetry::record(&LLMUsageStats {
                compactions: vec![event],
                ..Default::default()
            });
            let outcome = outcome?;
            if outcome.stale {
                continue;
            }
            self.publish().await;
            return Ok(true);
        }
        Ok(false)
    }

    /// Before the model call: sweeps deletions, runs the Hard compaction when one is due and
    /// assembles the prompt. `Err` only when the history cannot be read; the host then answers
    /// the way it did before memory.
    pub async fn prepare(
        &self,
        reader: &dyn HistoryReader,
        request: &TurnRequest<'_>,
    ) -> Result<PreparedTurn> {
        let delivery = UsageDelivery::new(self.engine.clone(), LLMUsageStats::default());
        let mut prepared =
            crate::telemetry::capture_into(delivery.buffer(), self.prepare_inner(reader, request))
                .await?;
        match self.engine.take_pending_usage().await {
            Ok(pending) => delivery.merge(&pending.take()),
            Err(error) => tracing::warn!("Reading pending memory usage failed: {error:#}"),
        }
        delivery
            .buffer()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .context = Some(ContextUsage {
            estimated_tokens: prepared.report.total_tokens,
            context_window: self.options.budget.context_window,
            reserved_output_tokens: self.options.budget.output,
            ..Default::default()
        });
        prepared.stats = delivery.snapshot();
        prepared.usage_delivery = delivery;
        Ok(prepared)
    }

    async fn prepare_inner(
        &self,
        reader: &dyn HistoryReader,
        request: &TurnRequest<'_>,
    ) -> Result<PreparedTurn> {
        let trigger = reader
            .message(request.trigger_seq)
            .await?
            .filter(|item| !item.deleted)
            .ok_or_else(|| anyhow!("The message #{} is gone", request.trigger_seq))?;
        if let Err(error) = self.sweep_deletions(reader, request.now).await
            && !is_busy(&error)
        {
            tracing::warn!("Following deletions failed: {error:#}");
        }
        let compacted = match self
            .compact(reader, request.trigger_seq, request, TurnPhase::BeforeCall)
            .await
        {
            Ok(ran) => ran,
            Err(error) => {
                if !is_busy(&error) {
                    tracing::warn!("The Hard compaction failed; answering uncompacted: {error:#}");
                }
                false
            }
        };
        let state = self.engine.stores().state.load().await?;
        let mut items = self
            .engine
            .live_items(reader, &state, request.trigger_seq)
            .await?;
        items.retain(|item| item.seq != request.trigger_seq);
        let plan = self
            .engine
            .plan(
                &items,
                &state,
                self.window(),
                request.open_task_seqs,
                TurnPhase::BeforeCall,
            )
            .await?;
        let summaries = self.engine.stores().segments.active().await?;
        let facts = self
            .engine
            .recall(&trigger.text, &self.options.budget, request.now)
            .await
            .unwrap_or_else(|error| {
                tracing::warn!("Recalling facts failed: {error:#}");
                Vec::new()
            });
        let recall_query = FactQuery {
            text: trigger.text.clone(),
            filter: FactFilter::default(),
            time_hint: None,
            limit: self.options.budget.facts_count,
            offset: 0,
            now: request.now,
            recency: Recency::default(),
            sort: Default::default(),
            geojson: None,
            radius_m: None,
            area: None,
        };
        let stores = self.engine.stores();
        let owner_budget = if stores.shared_facts.is_empty() {
            self.options.budget.about_state
        } else {
            self.options.budget.about_state / 2
        };
        let shared_budget = self.options.budget.about_state
            - if stores.owner_facts.is_some() {
                owner_budget
            } else {
                0
            };
        let owner_memory = super::recall::owner_memory(
            stores,
            &recall_query,
            true,
            self.options.timezone,
            owner_budget,
        )
        .await
        .unwrap_or_else(|error| {
            tracing::warn!("Recalling owner memory failed: {error:#}");
            Default::default()
        });
        let shared_memory = super::recall::shared_memory(
            stores,
            &recall_query,
            false,
            self.options.timezone,
            shared_budget,
        )
        .await
        .unwrap_or_else(|error| {
            tracing::warn!("Recalling shared memory failed: {error:#}");
            Default::default()
        });
        let about_you = [request.about_you, &owner_memory.text, &shared_memory.text]
            .into_iter()
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join("\n\n");
        let user_message = attributed_message(&trigger);
        let assembled = assemble(
            &PromptInput {
                system_and_tools: "",
                // Hosts assemble the final system prompt later. Keep its configured reserve
                // until they can supply a measured fixed-section size.
                tool_tokens: self
                    .options
                    .tool_tokens
                    .max(self.options.budget.system_tools),
                about_you: &about_you,
                summaries: &summaries,
                now_block: state.now.as_ref(),
                state_block: request.state_block,
                tail: &items,
                open_task_seqs: request.open_task_seqs,
                masking: &plan.masking,
                facts: &facts,
                user_message: &user_message,
                timezone: self.options.timezone,
            },
            &self.options.budget,
        );
        let text = |kind: SectionKind| {
            assembled
                .section(kind)
                .map(|section| section.text.trim().to_string())
                .filter(|text| !text.is_empty())
        };
        let context = [
            SectionKind::AboutYou,
            SectionKind::Summaries,
            SectionKind::State,
        ]
        .into_iter()
        .filter_map(text)
        .collect::<Vec<_>>()
        .join("\n\n");
        let prompt = [SectionKind::Memory, SectionKind::UserMessage]
            .into_iter()
            .filter_map(text)
            .collect::<Vec<_>>()
            .join("\n\n");
        let history = assembled
            .tail_seqs
            .iter()
            .filter_map(|seq| items.iter().find(|item| item.seq == *seq))
            .map(|item| TurnMessage {
                role: if item.role == Role::Assistant {
                    Role::Assistant
                } else {
                    Role::User
                },
                text: render_message(item, &plan.masking, self.options.timezone),
            })
            .collect();
        Ok(PreparedTurn {
            context,
            history,
            prompt,
            trigger,
            report: assembled.report,
            compacted,
            stats: LLMUsageStats::default(),
            usage_delivery: UsageDelivery::new(self.engine.clone(), LLMUsageStats::default()),
        })
    }

    /// After the reply is stored: the Soft compaction, fact mining of what the compactions did
    /// not cover, summaries that wait for a rewrite, deletions, publishing and periodic table
    /// maintenance. Runs under the engine's lease; another engine's work (`MemoryBusy`) defers
    /// the rest to the next turn. Dormant conversations wait for their next active turn.
    pub async fn after_turn(
        &self,
        reader: &dyn HistoryReader,
        request: &TurnRequest<'_>,
    ) -> AfterTurn {
        let delivery = UsageDelivery::new(self.engine.clone(), LLMUsageStats::default());
        let done = crate::telemetry::capture_into(
            delivery.buffer(),
            self.after_turn_inner(reader, request),
        )
        .await;
        if let Err(error) = self.engine.record_pending_usage(&delivery.take()).await {
            tracing::warn!("Persisting background memory usage was deferred: {error:#}");
        }
        done
    }

    async fn after_turn_inner(
        &self,
        reader: &dyn HistoryReader,
        request: &TurnRequest<'_>,
    ) -> AfterTurn {
        let latest = reader
            .latest()
            .await
            .ok()
            .flatten()
            .unwrap_or(request.trigger_seq)
            .max(request.trigger_seq);
        let mut done = AfterTurn::default();
        macro_rules! step {
            ($what:literal, $result:expr) => {
                match $result {
                    Ok(value) => Some(value),
                    Err(error) if is_busy(&error) => {
                        done.busy = true;
                        done.published += self.publish().await;
                        return done;
                    }
                    Err(error) => {
                        tracing::warn!(concat!($what, " failed: {:#}"), error);
                        None
                    }
                }
            };
        }
        if let Some((count, _)) = step!(
            "Following deletions",
            self.sweep_deletions(reader, request.now).await
        ) {
            done.deletions = count;
        }
        done.compacted = step!(
            "The background compaction",
            self.compact(reader, latest, request, TurnPhase::AfterTurn)
                .await
        )
        .unwrap_or(false);
        let state = step!(
            "Loading the memory state",
            self.engine.stores().state.load().await
        );
        if let Some(state) = state {
            let behind = latest.saturating_sub(state.mined_through.unwrap_or(0));
            if behind >= self.options.mine_after || !state.unmined.is_empty() {
                done.mined = step!(
                    "Mining facts",
                    self.engine.mine_idle(reader, latest, request.now).await
                )
                .is_some();
            }
            if !state.pending_rewrites.is_empty() {
                done.rewrote = step!(
                    "Rewriting summaries",
                    self.engine
                        .rewrite_pending(reader, self.window(), request.now)
                        .await
                )
                .is_some();
            }
        }
        done.published += self.publish().await;
        if let Some(Some(expired)) = step!(
            "Maintaining the memory tables",
            self.engine.maintain_if_due(request.now).await
        ) {
            done.optimized = true;
            done.expired_facts = expired;
        }
        done
    }
}

fn render_message(item: &HistoryItem, masking: &crate::compaction::Masking, tz: Tz) -> String {
    if item.speaker.is_some() {
        return render_tail_item(item, masking, tz);
    }
    let rendered = render_tail_item(item, masking, tz);
    let mut lines = rendered.lines();
    let _header = lines.next();
    let mut text = format!("#{} {}", item.seq, item.text);
    for line in lines {
        text.push('\n');
        text.push_str(line);
    }
    text
}

fn attributed_message(item: &HistoryItem) -> String {
    match &item.speaker {
        Some(speaker) => format!(
            "{} ({}) wrote: {}",
            speaker.label(),
            match item.role {
                Role::User => "your owner",
                Role::Assistant => "bot",
                _ => "participant statement",
            },
            item.text
        ),
        None => item.text.clone(),
    }
}

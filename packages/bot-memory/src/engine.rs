//! The engine a host drives per bot: plan each turn, compact (summaries, facts, merges, the Now
//! block), recall facts for the prompt, mine idle history and follow deletions. Compactions,
//! idle work and deletions run one at a time: within an engine behind a lock, across engines on
//! one store behind a lease in the saved state.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;

use chrono::{DateTime, Duration, Utc};
use chrono_tz::Tz;
use flow_like_types::tokio::sync::{Mutex, MutexGuard};
use flow_like_types::{Result, create_id};

use crate::compaction::{
    CompactionConfig, CompactionPlan, PlannerInput, TurnPhase, TurnPlan, plan_turn,
};
use crate::facts::{
    Evidence, ExtractionInput, Fact, Rejection, UpdateReport, evidence_trust, extract_facts,
    integrate_facts,
};
use crate::llm::MemoryLlm;
use crate::model::{FactSource, HistoryItem, Role, Seq, SeqSpan};
use crate::prompt::ModelBudget;
use crate::retrieval::{Recency, is_temporal_question, parse_time_range};
use crate::store::{CompactionState, FactFilter, FactQuery, Lease, MemoryStores, StateConflict};
use crate::summary::{
    NowInput, SegmentSummary, SpanSummaryInput, consolidate, merge_summaries, plan_merge,
    summarize_span, write_now_block,
};
use crate::tools::{HistoryReader, forget_facts};

/// What [`MemoryEngine::compact`] needs besides the plan.
pub struct CompactionInput<'a> {
    pub plan: &'a CompactionPlan,
    /// The live items the plan was made from.
    pub items: &'a [HistoryItem],
    /// Messages right before the evicted span (up to ten), context for extraction.
    pub preceding: &'a [HistoryItem],
    /// The host's structured state, for the Now block.
    pub state_block: &'a str,
    pub history_window: u32,
    pub now: DateTime<Utc>,
    /// The model that writes summaries, recorded with them.
    pub model: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CompactionOutcome {
    pub segments: Vec<SegmentSummary>,
    pub merged: Vec<SegmentSummary>,
    pub facts: UpdateReport,
    pub rejected: Vec<Rejection>,
    /// The plan was made from an older state (another compaction, idle mining or a deletion
    /// ran in between): nothing was done. Plan again.
    pub stale: bool,
    pub state: CompactionState,
}

impl CompactionOutcome {
    fn stale(state: CompactionState) -> Self {
        Self {
            segments: Vec::new(),
            merged: Vec::new(),
            facts: UpdateReport::default(),
            rejected: Vec::new(),
            stale: true,
            state,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct DeletionReport {
    pub facts_deleted: Vec<String>,
    pub facts_updated: Vec<String>,
    pub segments_rewritten: Vec<String>,
    /// Cleaned of the deleted messages without a model; [`MemoryEngine::rewrite_pending`]
    /// writes them again.
    pub segments_scrubbed: Vec<String>,
    pub segments_deleted: Vec<String>,
}

/// Another engine holds the memory's lease: it is compacting, mining or following a deletion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemoryBusy {
    pub until: DateTime<Utc>,
}

impl std::fmt::Display for MemoryBusy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Another engine is working on this bot's memory (its lease runs until {}); try again later",
            self.until
        )
    }
}

impl std::error::Error for MemoryBusy {}

pub struct MemoryEngine {
    stores: MemoryStores,
    llm: Arc<dyn MemoryLlm>,
    config: CompactionConfig,
    timezone: Tz,
    /// Names this engine in the state's lease.
    id: String,
    /// One compaction, idle run, deletion, tool write or maintenance run at a time.
    work: Mutex<()>,
}

/// The engine's turn and the store's lease, held by a tool write or maintenance run.
pub(crate) struct Leased<'a> {
    _work: MutexGuard<'a, ()>,
    claimed: CompactionState,
}

const MAX_MERGES_PER_COMPACTION: usize = 4;
/// How long a lease holds; the lease of an engine that crashed runs out.
const LEASE_MINUTES: i64 = 10;
/// The most messages one idle run mines; the next run continues after them.
const IDLE_BATCH: usize = 200;
const CLAIM_ATTEMPTS: usize = 3;

/// What mining produced, with the spans whose extraction failed.
#[derive(Default)]
struct Mined {
    report: UpdateReport,
    rejected: Vec<Rejection>,
    unmined: Vec<SeqSpan>,
}

impl Mined {
    fn absorb(&mut self, other: Mined) {
        self.report.absorb(other.report);
        self.rejected.extend(other.rejected);
        self.unmined.extend(other.unmined);
    }
}

impl MemoryEngine {
    pub fn new(
        stores: MemoryStores,
        llm: Arc<dyn MemoryLlm>,
        config: CompactionConfig,
        timezone: Tz,
    ) -> Self {
        Self {
            stores,
            llm,
            config,
            timezone,
            id: format!("engine_{}", create_id()),
            work: Mutex::new(()),
        }
    }

    pub fn stores(&self) -> &MemoryStores {
        &self.stores
    }

    pub fn config(&self) -> &CompactionConfig {
        &self.config
    }

    pub fn timezone(&self) -> Tz {
        self.timezone
    }

    pub(crate) fn llm(&self) -> &dyn MemoryLlm {
        self.llm.as_ref()
    }

    /// Takes the engine's turn (waiting for its other work) and the store's lease (refused with
    /// [`MemoryBusy`] while another engine holds it), as a compaction does.
    pub(crate) async fn lease(&self, now: DateTime<Utc>) -> Result<Leased<'_>> {
        let work = self.work.lock().await;
        let claimed = self.begin(now).await?;
        Ok(Leased {
            _work: work,
            claimed,
        })
    }

    /// Gives the lease back after work that left the state as it was; the work's result stands.
    pub(crate) async fn release<T>(&self, leased: Leased<'_>, result: Result<T>) -> Result<T> {
        if let Err(error) = self.finish(leased.claimed).await {
            tracing::warn!("Giving the memory lease back failed; it runs out by itself: {error:#}");
        }
        result
    }

    /// Idle maintenance under the lease: embeds facts and summaries that have no vector from the
    /// current embedder yet, folds new rows into the indexes and drops versions past Lance's
    /// retention. Refused with [`MemoryBusy`] while another engine works on the memory.
    pub async fn optimize(&self, now: DateTime<Utc>) -> Result<()> {
        let leased = self.lease(now).await?;
        let result = self.stores.optimize().await;
        self.release(leased, result).await
    }

    /// The items the prompt shows: everything after the last compaction plus the retained ones.
    pub async fn live_items(
        &self,
        reader: &dyn HistoryReader,
        state: &CompactionState,
        latest: Seq,
    ) -> Result<Vec<HistoryItem>> {
        let from = state.compacted_through.map_or(0, |through| through + 1);
        let mut items = Vec::new();
        for seq in &state.retained {
            if *seq < from
                && let Some(item) = reader.message(*seq).await?
                && !item.deleted
            {
                items.push(item);
            }
        }
        if from <= latest {
            items.extend(reader.range(SeqSpan::new(from, latest), usize::MAX).await?);
        }
        Ok(items)
    }

    /// Masks old tool results, measures the turn and plans a compaction when one is due. The
    /// plan carries the state's revision: once the state moved on, [`MemoryEngine::compact`]
    /// returns it as stale.
    pub async fn plan(
        &self,
        items: &[HistoryItem],
        state: &CompactionState,
        history_window: u32,
        open_task_seqs: &[Seq],
        phase: TurnPhase,
    ) -> Result<TurnPlan> {
        let summaries = self.stores.segments.active().await?;
        let mut turn = plan_turn(
            &PlannerInput {
                items,
                compacted_through: state.compacted_through,
                history_window,
                summary_tokens: summaries.iter().map(SegmentSummary::tokens).sum(),
                now_tokens: state.now.as_ref().map_or(0, |block| block.tokens()),
                open_task_seqs,
                phase,
            },
            &self.config,
        );
        if let Some(plan) = turn.compaction.as_mut() {
            plan.base_revision = state.revision;
        }
        Ok(turn)
    }

    /// Adds messages to the history index (also refreshes edited ones and drops tombstones).
    pub async fn index(&self, items: &[HistoryItem]) -> Result<()> {
        self.stores.history.index(items).await
    }

    /// Claims the lease on `state` by saving it; refused while another engine holds it.
    async fn claim(&self, state: CompactionState, now: DateTime<Utc>) -> Result<CompactionState> {
        if let Some(lease) = state
            .lease
            .as_ref()
            .filter(|lease| lease.holder != self.id && lease.until > now)
        {
            return Err(MemoryBusy { until: lease.until }.into());
        }
        let claimed = self
            .stores
            .state
            .save(&CompactionState {
                lease: Some(Lease {
                    holder: self.id.clone(),
                    until: now + Duration::minutes(LEASE_MINUTES),
                }),
                ..state
            })
            .await?;
        self.stores.refresh().await?;
        Ok(claimed)
    }

    /// Loads and claims the state, once more when another writer saved in between.
    async fn begin(&self, now: DateTime<Utc>) -> Result<CompactionState> {
        let mut attempt = 1;
        loop {
            let state = self.stores.state.load().await?;
            match self.claim(state, now).await {
                Err(error) if error.is::<StateConflict>() && attempt < CLAIM_ATTEMPTS => {
                    attempt += 1;
                }
                claimed => return claimed,
            }
        }
    }

    /// Saves the work and gives the lease back.
    async fn finish(&self, state: CompactionState) -> Result<CompactionState> {
        self.stores
            .state
            .save(&CompactionState {
                lease: None,
                ..state
            })
            .await
    }

    /// After a failure the lease goes back and the state stays as it was claimed.
    async fn settle<T>(&self, claimed: &CompactionState, result: Result<T>) -> Result<T> {
        if result.is_err()
            && let Err(error) = self.finish(claimed.clone()).await
        {
            tracing::warn!("Giving the memory lease back failed; it runs out by itself: {error:#}");
        }
        result
    }

    async fn mine(
        &self,
        span: &[HistoryItem],
        preceding: &[HistoryItem],
        summary: Option<&str>,
        segment_id: Option<&str>,
        now: DateTime<Utc>,
    ) -> Result<Mined> {
        if span.is_empty() {
            return Ok(Mined::default());
        }
        let outcome = extract_facts(
            self.llm.as_ref(),
            &ExtractionInput {
                span,
                preceding,
                summary,
                timezone: self.timezone,
                segment_id,
                now,
                chunk_tokens: self.config.chunk_tokens,
                preceding_messages: self.config.preceding_messages,
            },
        )
        .await?;
        let report = integrate_facts(
            &self.stores.facts,
            self.llm.as_ref(),
            outcome.facts,
            self.timezone,
            now,
        )
        .await?;
        Ok(Mined {
            report,
            rejected: outcome.rejected,
            unmined: outcome.unmined,
        })
    }

    /// Runs a planned compaction: a level-0 summary per segment, facts from every span not mined
    /// yet, merges one level up while summaries exceed their share, a new Now block, then the
    /// new state. A plan made before the state last changed comes back `stale`, without effect.
    /// Safe to retry after a failure: summaries already written are reused and equal facts
    /// merge.
    pub async fn compact(&self, input: CompactionInput<'_>) -> Result<CompactionOutcome> {
        let _work = self.work.lock().await;
        let current = self.stores.state.load().await?;
        if current.revision != input.plan.base_revision {
            return Ok(CompactionOutcome::stale(current));
        }
        let claimed = match self.claim(current, input.now).await {
            Err(error) if error.is::<StateConflict>() => {
                return Ok(CompactionOutcome::stale(self.stores.state.load().await?));
            }
            claimed => claimed?,
        };
        let result = self.compact_claimed(&input, claimed.clone()).await;
        self.settle(&claimed, result).await
    }

    async fn compact_claimed(
        &self,
        input: &CompactionInput<'_>,
        mut state: CompactionState,
    ) -> Result<CompactionOutcome> {
        let tz = self.timezone;
        let window = input.history_window;
        let mut live: Vec<&HistoryItem> = input.items.iter().filter(|item| !item.deleted).collect();
        live.sort_by_key(|item| item.seq);
        self.drop_leftovers(state.compacted_through, &input.plan.segments)
            .await?;
        let mut previous = self
            .stores
            .segments
            .active()
            .await?
            .into_iter()
            .rfind(|summary| summary.span.to < input.plan.evict.from);
        let mut segments = Vec::new();
        let mut mined = Mined::default();

        for span in &input.plan.segments {
            if !live.iter().any(|item| span.contains(item.seq)) {
                state.mined_through = Some(
                    state
                        .mined_through
                        .map_or(span.to, |mined| mined.max(span.to)),
                );
                continue;
            }
            let existing = self
                .stores
                .segments
                .get(&SegmentSummary::id_for(0, *span))
                .await?;
            let summary = match existing {
                Some(written) => written,
                None => {
                    let summary = summarize_span(
                        self.llm.as_ref(),
                        &SpanSummaryInput {
                            items: input.items,
                            span: *span,
                            previous: previous.as_ref(),
                            timezone: tz,
                            budget_tokens: self.config.segment_summary_budget(window),
                            chunk_tokens: self.config.chunk_tokens,
                            model: input.model.clone(),
                            now: input.now,
                        },
                    )
                    .await?;
                    self.stores
                        .segments
                        .put(std::slice::from_ref(&summary))
                        .await?;
                    summary
                }
            };
            if !summary.missing.is_empty() && !state.pending_rewrites.contains(&summary.id) {
                state.pending_rewrites.push(summary.id.clone());
            }

            let unmined: Vec<HistoryItem> = live
                .iter()
                .filter(|item| {
                    span.contains(item.seq)
                        && state.mined_through.is_none_or(|mined| item.seq > mined)
                })
                .map(|item| (*item).clone())
                .collect();
            let context: BTreeMap<Seq, HistoryItem> = input
                .preceding
                .iter()
                .chain(live.iter().copied())
                .filter(|item| !item.deleted && item.seq < span.from)
                .map(|item| (item.seq, item.clone()))
                .collect();
            let context: Vec<HistoryItem> = context.into_values().collect();
            let keep_from = context.len().saturating_sub(self.config.preceding_messages);
            let rendered = summary.render(tz);
            mined.absorb(
                self.mine(
                    &unmined,
                    &context[keep_from..],
                    Some(&rendered),
                    Some(&summary.id),
                    input.now,
                )
                .await?,
            );
            state.mined_through = Some(
                state
                    .mined_through
                    .map_or(span.to, |mined| mined.max(span.to)),
            );
            previous = Some(summary.clone());
            segments.push(summary);
        }

        let evicted: Vec<HistoryItem> = live
            .iter()
            .filter(|item| input.plan.evict.contains(item.seq))
            .map(|item| (*item).clone())
            .collect();
        self.stores.history.index(&evicted).await?;

        let merged = self
            .merge_until_small(window, input.model.clone(), input.now)
            .await?;

        let tail: Vec<HistoryItem> = live
            .iter()
            .filter(|item| item.seq >= input.plan.tail_from)
            .map(|item| (*item).clone())
            .collect();
        let latest = self.stores.segments.active().await?.into_iter().last();
        let now_block = write_now_block(
            self.llm.as_ref(),
            &NowInput {
                tail: &tail,
                latest_summary: latest.as_ref(),
                state_block: input.state_block,
                timezone: tz,
                budget_tokens: self.config.now_budget(window),
                chunk_tokens: self.config.chunk_tokens,
            },
        )
        .await;
        match now_block {
            Ok(block) => state.now = Some(block),
            Err(error) => {
                tracing::warn!("Writing the Now block failed; keeping the previous one: {error:#}")
            }
        }

        state.compacted_through = Some(
            state
                .compacted_through
                .map_or(input.plan.compacted_through(), |through| {
                    through.max(input.plan.compacted_through())
                }),
        );
        state.retained = input.plan.retained.clone();
        state.unmined.extend(mined.unmined);
        state.compactions += 1;
        state.updated_at = Some(input.now);
        let state = self.finish(state).await?;
        Ok(CompactionOutcome {
            segments,
            merged,
            facts: mined.report,
            rejected: mined.rejected,
            stale: false,
            state,
        })
    }

    /// Removes what an earlier run that failed or was cut off left behind: active summaries
    /// reaching past `compacted_through` that this plan does not reuse. A merged one frees its
    /// children first, which are checked in turn. Safe because no other run is in flight.
    async fn drop_leftovers(&self, through: Option<Seq>, reuse: &[SeqSpan]) -> Result<()> {
        loop {
            let leftovers: Vec<SegmentSummary> = self
                .stores
                .segments
                .active()
                .await?
                .into_iter()
                .filter(|summary| {
                    through.is_none_or(|through| summary.span.to > through)
                        && !(summary.level == 0 && reuse.contains(&summary.span))
                })
                .collect();
            if leftovers.is_empty() {
                return Ok(());
            }
            for summary in &leftovers {
                self.stores
                    .segments
                    .set_parent(&summary.children, None)
                    .await?;
            }
            let ids: Vec<String> = leftovers.into_iter().map(|summary| summary.id).collect();
            self.stores.segments.delete(&ids).await?;
        }
    }

    /// Merges active summaries one level up ([`plan_merge`]) while they exceed their share of H.
    async fn merge_until_small(
        &self,
        window: u32,
        model: Option<String>,
        now: DateTime<Utc>,
    ) -> Result<Vec<SegmentSummary>> {
        let budget = self.merge_budget(window);
        let mut merged = Vec::new();
        for _ in 0..MAX_MERGES_PER_COMPACTION {
            let active = self.stores.segments.active().await?;
            let Some(ids) = plan_merge(&active, window, &self.config) else {
                break;
            };
            let children: Vec<SegmentSummary> = active
                .into_iter()
                .filter(|summary| ids.contains(&summary.id))
                .collect();
            let summary = merge_summaries(
                self.llm.as_ref(),
                &children,
                self.timezone,
                budget,
                model.clone(),
                now,
            )
            .await?;
            self.stores
                .segments
                .put(std::slice::from_ref(&summary))
                .await?;
            self.stores
                .segments
                .set_parent(&ids, Some(&summary.id))
                .await?;
            merged.push(summary);
        }
        Ok(merged)
    }

    fn merge_budget(&self, window: u32) -> u32 {
        self.config
            .segment_summary_budget(window)
            .max(self.config.merge_threshold(window) / 3)
    }

    /// Idle extraction ("dreaming"): facts from the messages after the last mined one through
    /// `latest` (at most 200 a run; the next run goes on from there) and from spans whose
    /// extraction failed or must be redone. Reads the history itself, so no gap is skipped.
    pub async fn mine_idle(
        &self,
        reader: &dyn HistoryReader,
        latest: Seq,
        now: DateTime<Utc>,
    ) -> Result<(UpdateReport, Vec<Rejection>)> {
        let _work = self.work.lock().await;
        let claimed = self.begin(now).await?;
        let result = self
            .mine_claimed(reader, latest, now, claimed.clone())
            .await;
        self.settle(&claimed, result).await
    }

    async fn mine_claimed(
        &self,
        reader: &dyn HistoryReader,
        latest: Seq,
        now: DateTime<Utc>,
        mut state: CompactionState,
    ) -> Result<(UpdateReport, Vec<Rejection>)> {
        let summary = state.now.as_ref().map(|block| block.text.clone());
        let mut mined = Mined::default();
        for span in std::mem::take(&mut state.unmined) {
            let items = reader.range(span, usize::MAX).await?;
            let preceding = reader
                .before(span.from, self.config.preceding_messages)
                .await?;
            mined.absorb(
                self.mine(&items, &preceding, summary.as_deref(), None, now)
                    .await?,
            );
        }
        let from = state.mined_through.map_or(0, |mined| mined + 1);
        if from <= latest {
            let items = reader.range(SeqSpan::new(from, latest), IDLE_BATCH).await?;
            let preceding = reader.before(from, self.config.preceding_messages).await?;
            mined.absorb(
                self.mine(&items, &preceding, summary.as_deref(), None, now)
                    .await?,
            );
            let through = match items.last() {
                Some(last) if items.len() == IDLE_BATCH => last.seq,
                _ => latest,
            };
            state.mined_through = Some(
                state
                    .mined_through
                    .map_or(through, |mined| mined.max(through)),
            );
        }
        state.unmined = mined.unmined;
        state.updated_at = Some(now);
        self.finish(state).await?;
        Ok((mined.report, mined.rejected))
    }

    /// The facts to inject before the user's message: hybrid search, events weighted by
    /// recency, time cues as a ranking hint (lasting facts stay eligible), superseded facts only
    /// for questions about how things were.
    pub async fn recall(
        &self,
        query: &str,
        budget: &ModelBudget,
        now: DateTime<Utc>,
    ) -> Result<Vec<Fact>> {
        let found = self
            .stores
            .facts
            .search(&FactQuery {
                text: query.to_string(),
                filter: FactFilter {
                    include_superseded: is_temporal_question(query),
                    ..FactFilter::default()
                },
                time_hint: parse_time_range(query, now, self.timezone),
                limit: budget.facts_count,
                offset: 0,
                now,
                recency: Recency::default(),
            })
            .await?;
        Ok(found.into_iter().map(|scored| scored.fact).collect())
    }

    /// Follows deleted messages. Facts lose that evidence and keep only the trust the rest
    /// carries; a fact goes when none of the rest backs its whole text, or a profile fact or
    /// preference keeps no word of the user (what the rest said is mined again, and facts a gone
    /// one had superseded are current again). The history index drops the messages, every
    /// summary covering them is written again from the remaining history (without a model it is
    /// cleaned and queued for [`MemoryEngine::rewrite_pending`]), the Now block forgets lines
    /// citing them, and old table versions are pruned so the content leaves the disk.
    pub async fn on_messages_deleted(
        &self,
        seqs: &[Seq],
        reader: &dyn HistoryReader,
        history_window: u32,
        now: DateTime<Utc>,
    ) -> Result<DeletionReport> {
        self.follow_deletions(seqs, None, reader, history_window, now)
            .await
    }

    /// [`MemoryEngine::on_messages_deleted`] that also records the position the deletions were
    /// read up to (`state.deletions_through`), so the next sweep starts after it.
    pub async fn follow_deletions(
        &self,
        seqs: &[Seq],
        through: Option<Seq>,
        reader: &dyn HistoryReader,
        history_window: u32,
        now: DateTime<Utc>,
    ) -> Result<DeletionReport> {
        let _work = self.work.lock().await;
        let claimed = self.begin(now).await?;
        let result = self
            .follow_deletion(seqs, through, reader, history_window, now, claimed.clone())
            .await;
        let report = self.settle(&claimed, result).await?;
        if let Err(error) = self.stores.prune().await {
            tracing::warn!(
                "Pruning old memory versions after a deletion failed; optimize drops them later: {error:#}"
            );
        }
        Ok(report)
    }

    async fn follow_deletion(
        &self,
        seqs: &[Seq],
        through: Option<Seq>,
        reader: &dyn HistoryReader,
        window: u32,
        now: DateTime<Utc>,
        mut state: CompactionState,
    ) -> Result<DeletionReport> {
        let deleted: BTreeSet<Seq> = seqs.iter().copied().collect();
        let mut report = DeletionReport::default();
        for seq in self.drop_evidence(&deleted, reader, &mut report).await? {
            state.unmined.push(SeqSpan::new(seq, seq));
        }
        self.stores.history.remove(seqs).await?;
        for summary in self.summaries_covering(&deleted).await? {
            if summary.level == 0 {
                self.rewrite_level_zero(&summary, &deleted, reader, window, now, &mut report)
                    .await?;
            } else {
                self.remerge(&summary, window, now, &mut report).await?;
            }
        }
        for id in &report.segments_scrubbed {
            if !state.pending_rewrites.contains(id) {
                state.pending_rewrites.push(id.clone());
            }
        }
        state.retained.retain(|seq| !deleted.contains(seq));
        state.now = state.now.map(|block| block.without_seqs(seqs));
        state.deletions_through = state.deletions_through.max(through);
        state.updated_at = Some(now);
        self.finish(state).await?;
        Ok(report)
    }

    /// Removes the deleted positions from every fact's evidence and recomputes its trust from
    /// what remains. Returns the positions whose facts must be mined again.
    async fn drop_evidence(
        &self,
        deleted: &BTreeSet<Seq>,
        reader: &dyn HistoryReader,
        report: &mut DeletionReport,
    ) -> Result<Vec<Seq>> {
        let seqs: Vec<Seq> = deleted.iter().copied().collect();
        let mut roles: HashMap<Seq, Option<Role>> = HashMap::new();
        let mut gone = Vec::new();
        let mut remine = BTreeSet::new();
        for fact in self.stores.facts.with_evidence(&seqs).await? {
            let mut remaining = fact.clone();
            remaining
                .evidence
                .retain(|evidence| !deleted.contains(&evidence.seq));
            let trust = self
                .remaining_trust(&remaining.evidence, reader, &mut roles)
                .await?;
            let unsupported = remaining.evidence.iter().all(|evidence| evidence.partial)
                || (fact.kind.is_personal() && trust != Some(FactSource::UserStated));
            if unsupported {
                remine.extend(remaining.evidence_seqs());
                report.facts_deleted.push(fact.id.clone());
                gone.push(fact);
            } else {
                if let Some(trust) = trust {
                    remaining.source = remaining.source.min(trust);
                }
                self.stores.facts.update_metadata(&remaining).await?;
                report.facts_updated.push(remaining.id);
            }
        }
        forget_facts(&self.stores, &gone, true).await?;
        Ok(remine.into_iter().collect())
    }

    /// The most trust any of `evidence` still carries, by the roles in the history.
    async fn remaining_trust(
        &self,
        evidence: &[Evidence],
        reader: &dyn HistoryReader,
        roles: &mut HashMap<Seq, Option<Role>>,
    ) -> Result<Option<FactSource>> {
        let mut best = None;
        for evidence in evidence {
            let role = match roles.get(&evidence.seq) {
                Some(role) => *role,
                None => {
                    let role = reader
                        .message(evidence.seq)
                        .await?
                        .filter(|item| !item.deleted)
                        .map(|item| item.role);
                    roles.insert(evidence.seq, role);
                    role
                }
            };
            if let Some(role) = role {
                best = best.max(Some(evidence_trust(role, evidence.part)));
            }
        }
        Ok(best)
    }

    /// Every summary whose range holds a deleted position, lowest level first, so merges are
    /// redone from children that were already rewritten.
    async fn summaries_covering(&self, deleted: &BTreeSet<Seq>) -> Result<Vec<SegmentSummary>> {
        let mut covering: Vec<SegmentSummary> = Vec::new();
        for seq in deleted {
            for summary in self.stores.segments.covering(*seq).await? {
                if !covering.iter().any(|known| known.id == summary.id) {
                    covering.push(summary);
                }
            }
        }
        covering.sort_by_key(|summary| summary.level);
        Ok(covering)
    }

    /// A level-0 summary written again from the remaining history; `None` when every message in
    /// its range is gone.
    async fn summarize_again(
        &self,
        summary: &SegmentSummary,
        reader: &dyn HistoryReader,
        window: u32,
        now: DateTime<Utc>,
    ) -> Result<Option<SegmentSummary>> {
        let items = reader.range(summary.span, usize::MAX).await?;
        if items.iter().all(|item| item.deleted) {
            return Ok(None);
        }
        let mut rewritten = summarize_span(
            self.llm.as_ref(),
            &SpanSummaryInput {
                items: &items,
                span: summary.span,
                previous: None,
                timezone: self.timezone,
                budget_tokens: self.config.segment_summary_budget(window),
                chunk_tokens: self.config.chunk_tokens,
                model: summary.model.clone(),
                now,
            },
        )
        .await?;
        rewritten.version = summary.version + 1;
        rewritten.parent = summary.parent.clone();
        Ok(Some(rewritten))
    }

    async fn rewrite_level_zero(
        &self,
        summary: &SegmentSummary,
        deleted: &BTreeSet<Seq>,
        reader: &dyn HistoryReader,
        window: u32,
        now: DateTime<Utc>,
        report: &mut DeletionReport,
    ) -> Result<()> {
        match self.summarize_again(summary, reader, window, now).await {
            Ok(Some(rewritten)) if rewritten.missing.is_empty() => {
                self.stores
                    .segments
                    .put(std::slice::from_ref(&rewritten))
                    .await?;
                report.segments_rewritten.push(rewritten.id);
            }
            Ok(None) => {
                self.stores
                    .segments
                    .delete(std::slice::from_ref(&summary.id))
                    .await?;
                report.segments_deleted.push(summary.id.clone());
            }
            outcome => {
                if let Err(error) = outcome {
                    tracing::warn!(
                        "Writing summary {} again after a deletion failed; it is cleaned now and rewritten later: {error:#}",
                        summary.id
                    );
                }
                self.stores
                    .segments
                    .put(std::slice::from_ref(&summary.scrubbed(deleted)))
                    .await?;
                report.segments_scrubbed.push(summary.id.clone());
            }
        }
        Ok(())
    }

    /// The summary with its missing ranges asked again from the history; what fails again stays
    /// missing.
    async fn fill_missing(
        &self,
        summary: &SegmentSummary,
        reader: &dyn HistoryReader,
        window: u32,
        now: DateTime<Utc>,
    ) -> Result<SegmentSummary> {
        let budget = self.config.segment_summary_budget(window);
        let mut filled = SegmentSummary {
            missing: Vec::new(),
            version: summary.version + 1,
            ..summary.clone()
        };
        for span in &summary.missing {
            let items = reader.range(*span, usize::MAX).await?;
            if items.iter().all(|item| item.deleted) {
                continue;
            }
            let piece = summarize_span(
                self.llm.as_ref(),
                &SpanSummaryInput {
                    items: &items,
                    span: *span,
                    previous: None,
                    timezone: self.timezone,
                    budget_tokens: budget,
                    chunk_tokens: self.config.chunk_tokens,
                    model: summary.model.clone(),
                    now,
                },
            )
            .await?;
            filled.body.absorb(piece.body);
            filled.missing.extend(piece.missing);
            filled.user_seqs.extend(piece.user_seqs);
        }
        filled.body = consolidate(filled.body);
        filled.user_seqs.sort_unstable();
        filled.user_seqs.dedup();
        filled.trim_to(budget);
        Ok(filled)
    }

    async fn remerge(
        &self,
        summary: &SegmentSummary,
        window: u32,
        now: DateTime<Utc>,
        report: &mut DeletionReport,
    ) -> Result<()> {
        let children = self.stores.segments.children(&summary.id).await?;
        if children.is_empty() {
            self.stores
                .segments
                .delete(std::slice::from_ref(&summary.id))
                .await?;
            report.segments_deleted.push(summary.id.clone());
            return Ok(());
        }
        let mut rewritten = merge_summaries(
            self.llm.as_ref(),
            &children,
            self.timezone,
            self.merge_budget(window),
            summary.model.clone(),
            now,
        )
        .await?;
        rewritten.id = summary.id.clone();
        rewritten.version = summary.version + 1;
        rewritten.parent = summary.parent.clone();
        self.stores
            .segments
            .put(std::slice::from_ref(&rewritten))
            .await?;
        report.segments_rewritten.push(rewritten.id);
        Ok(())
    }

    /// Idle work for the summaries in `state.pending_rewrites`: ranges a summary misses (the
    /// model gave no usable answer) are asked again; a summary cleaned after a deletion without
    /// a model is written again from the history. Summaries above a completed one are merged
    /// again. What fails stays queued for the next run.
    pub async fn rewrite_pending(
        &self,
        reader: &dyn HistoryReader,
        history_window: u32,
        now: DateTime<Utc>,
    ) -> Result<DeletionReport> {
        let _work = self.work.lock().await;
        let claimed = self.begin(now).await?;
        let result = self
            .rewrite_claimed(reader, history_window, now, claimed.clone())
            .await;
        self.settle(&claimed, result).await
    }

    async fn rewrite_claimed(
        &self,
        reader: &dyn HistoryReader,
        window: u32,
        now: DateTime<Utc>,
        mut state: CompactionState,
    ) -> Result<DeletionReport> {
        let mut report = DeletionReport::default();
        let mut waiting = Vec::new();
        for id in std::mem::take(&mut state.pending_rewrites) {
            let Some(summary) = self.stores.segments.get(&id).await? else {
                continue;
            };
            let filling = !summary.missing.is_empty();
            let outcome = if filling {
                self.fill_missing(&summary, reader, window, now)
                    .await
                    .map(Some)
            } else {
                self.summarize_again(&summary, reader, window, now).await
            };
            match outcome {
                Ok(Some(partial)) if !partial.missing.is_empty() => {
                    if filling {
                        self.stores
                            .segments
                            .put(std::slice::from_ref(&partial))
                            .await?;
                    }
                    waiting.push(id);
                }
                Ok(Some(rewritten)) => {
                    self.stores
                        .segments
                        .put(std::slice::from_ref(&rewritten))
                        .await?;
                    report.segments_rewritten.push(id);
                    let mut above = summary.parent.clone();
                    while let Some(parent) = above {
                        let Some(parent) = self.stores.segments.get(&parent).await? else {
                            break;
                        };
                        self.remerge(&parent, window, now, &mut report).await?;
                        above = parent.parent;
                    }
                }
                Ok(None) => {
                    self.stores
                        .segments
                        .delete(std::slice::from_ref(&id))
                        .await?;
                    report.segments_deleted.push(id);
                }
                Err(error) => {
                    tracing::warn!("Writing summary {id} again failed; it stays queued: {error:#}");
                    waiting.push(id);
                }
            }
        }
        state.pending_rewrites = waiting;
        state.updated_at = Some(now);
        self.finish(state).await?;
        Ok(report)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::MemoryStores;
    use crate::tools::{
        HistoryReadArgs, MemoryForgetArgs, MemorySaveArgs, MemorySearchArgs, WriteGate,
        history_read, memory_forget, memory_save, memory_search,
    };

    fn send<T: Send>(_: &T) {}

    /// Hosts spawn compactions and run turns on multi-threaded runtimes: every future the
    /// engine and the tools return must be `Send`. This only has to compile.
    #[allow(dead_code)]
    fn futures_are_send(
        engine: &MemoryEngine,
        stores: &MemoryStores,
        reader: &dyn HistoryReader,
        input: CompactionInput<'_>,
        budget: &ModelBudget,
        evidence: &HistoryItem,
        now: DateTime<Utc>,
    ) {
        let save = MemorySaveArgs {
            text: "text".into(),
            evidence_seq: None,
            kind: None,
            subject: None,
            valid_from: None,
            valid_to: None,
        };
        send(&memory_save(
            engine,
            &save,
            evidence,
            WriteGate::default(),
            now,
        ));
        let forget = MemoryForgetArgs { id: "f_x".into() };
        send(&memory_forget(engine, &forget, WriteGate::default(), now));
        send(&engine.optimize(now));
        send(&engine.compact(input));
        send(&engine.plan(
            &[],
            &CompactionState::default(),
            1_000,
            &[],
            TurnPhase::Idle,
        ));
        send(&engine.recall("query", budget, now));
        send(&engine.mine_idle(reader, 10, now));
        send(&engine.on_messages_deleted(&[1], reader, 1_000, now));
        send(&engine.rewrite_pending(reader, 1_000, now));
        send(&engine.live_items(reader, &CompactionState::default(), 10));
        let search = MemorySearchArgs {
            query: "query".into(),
            kind: None,
            time_range: None,
            include_superseded: None,
            limit: None,
            cursor: None,
        };
        send(&memory_search(stores, &search, now, Tz::UTC));
        let read = HistoryReadArgs {
            at: "#1".into(),
            before: 1,
            after: 1,
            max_tokens: None,
        };
        send(&history_read(reader, &read, Tz::UTC));
    }
}

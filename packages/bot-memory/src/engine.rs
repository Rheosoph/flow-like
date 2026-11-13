//! The engine a host drives per bot: plan each turn, compact (summaries, facts, merges, the Now
//! block), recall facts for the prompt, mine idle history and follow deletions. Compactions,
//! model and embedding work prepares private drafts. Only revision-validated storage commits
//! run under the shared lease, so a slow model does not block explicit saves or forgets.

mod commit;
mod draft;
mod maintenance;
mod reset;
mod telemetry;

pub(crate) use commit::CommitOutcome;

use draft::SegmentDraft;

use std::collections::BTreeMap;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration as WaitDuration;

use chrono::{DateTime, Duration, Utc};
use chrono_tz::Tz;
use flow_like_types::tokio::sync::Mutex;
use flow_like_types::tokio::time::{Instant, sleep};
use flow_like_types::{Result, create_id};

use crate::compaction::{
    CompactionConfig, CompactionPlan, PlannerInput, TurnPhase, TurnPlan, plan_turn,
};
use crate::facts::{
    ExtractionInput, Fact, Rejection, UpdateReport, apply_prepared_facts, extract_facts,
    prepare_facts,
};
use crate::llm::MemoryLlm;
use crate::model::{HistoryItem, Seq, SeqSpan};
use crate::prompt::ModelBudget;
use crate::retrieval::{Recency, is_temporal_question, parse_time_range};
use crate::store::{CompactionState, FactFilter, FactQuery, Lease, MemoryStores, StateConflict};
use crate::summary::{
    NowInput, SegmentSummary, SpanSummaryInput, merge_summaries, plan_merge, summarize_span,
    write_now_block,
};
use crate::tools::HistoryReader;

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

#[derive(Clone)]
pub struct MemoryEngine {
    stores: Arc<MemoryStores>,
    llm: Arc<dyn MemoryLlm>,
    config: CompactionConfig,
    timezone: Tz,
    /// Names this engine in the state's lease.
    id: String,
    /// Serializes storage commits within this engine. Preparation never holds it.
    work: Arc<Mutex<()>>,
}

const MAX_MERGES_PER_COMPACTION: usize = 4;
/// How long a lease holds; the lease of an engine that crashed runs out.
const LEASE_MINUTES: i64 = 10;
/// The most messages one idle run mines; the next run continues after them.
const IDLE_BATCH: usize = 200;
const CLAIM_ATTEMPTS: usize = 3;
const WRITE_LEASE_WAIT: WaitDuration = WaitDuration::from_secs(30);
const WRITE_LEASE_POLL: WaitDuration = WaitDuration::from_millis(250);
const MAINTENANCE_HOURS: i64 = 6;
const MAINTENANCE_HEARTBEAT: WaitDuration = WaitDuration::from_secs(120);

/// What mining produced, with the spans whose extraction failed.
#[derive(Default)]
struct Mined {
    candidates: Vec<Fact>,
    rejected: Vec<Rejection>,
    unmined: Vec<SeqSpan>,
}

impl Mined {
    fn absorb(&mut self, other: Mined) {
        self.candidates.extend(other.candidates);
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
            stores: Arc::new(stores),
            llm,
            config,
            timezone,
            id: format!("engine_{}", create_id()),
            work: Arc::new(Mutex::new(())),
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

    /// Runs expiry cleanup, embedding and index maintenance, resetting the six-hour cadence.
    pub async fn optimize(&self, now: DateTime<Utc>) -> Result<()> {
        self.maintain(now, false).await.map(|_| ())
    }

    /// Returns the number of expired facts deleted, or `None` when maintenance is not due.
    pub async fn maintain_if_due(&self, now: DateTime<Utc>) -> Result<Option<usize>> {
        self.maintain(now, true).await
    }

    async fn maintain(&self, now: DateTime<Utc>, only_if_due: bool) -> Result<Option<usize>> {
        let started = Instant::now();
        let state = self.background_snapshot(now).await?;
        if only_if_due
            && state.last_maintenance_at.is_some_and(|last| {
                now.signed_duration_since(last) < Duration::hours(MAINTENANCE_HOURS)
            })
        {
            return Ok(None);
        }
        let revision = state.revision;
        let prepared = self.stores.prepare_optimization(now).await;
        let commit_now = now + Duration::from_std(started.elapsed())?;
        let (expired, _) = self
            .commit(revision, commit_now, move |stores, mut state| async move {
                let expired = stores.facts.purge_expired(now).await?;
                stores.apply_optimization(prepared).await?;
                state.last_maintenance_at = Some(now);
                state.updated_at = Some(now);
                Ok((expired, state))
            })
            .await?
            .applied(revision)?;
        Ok(Some(expired))
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
        if let Err(error) = self.stores.refresh().await {
            if let Err(release_error) = self.finish(claimed.clone()).await {
                tracing::warn!(
                    "Releasing memory after a refresh failure failed: {release_error:#}"
                );
            }
            return Err(error);
        }
        Ok(claimed)
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
        Ok(Mined {
            candidates: outcome.facts,
            rejected: outcome.rejected,
            unmined: outcome.unmined,
        })
    }

    /// Builds a private draft, then commits it only if no other memory mutation intervened.
    /// Stale drafts have no storage effects; the host can plan again from the returned state.
    pub async fn compact(&self, input: CompactionInput<'_>) -> Result<CompactionOutcome> {
        let started = Instant::now();
        let current = self.background_snapshot(input.now).await?;
        if current.revision != input.plan.base_revision {
            return Ok(CompactionOutcome::stale(current));
        }
        self.prepare_compaction(&input, current, started).await
    }

    async fn prepare_compaction(
        &self,
        input: &CompactionInput<'_>,
        mut state: CompactionState,
        started: Instant,
    ) -> Result<CompactionOutcome> {
        let tz = self.timezone;
        let window = input.history_window;
        let mut live: Vec<&HistoryItem> = input.items.iter().filter(|item| !item.deleted).collect();
        live.sort_by_key(|item| item.seq);
        let revision = state.revision;
        let mut draft = match self
            .check_preparation(revision, SegmentDraft::load(&self.stores.segments))
            .await
        {
            Ok(draft) => draft,
            Err(error) if error.is::<StateConflict>() => {
                return Ok(CompactionOutcome::stale(self.stores.state.load().await?));
            }
            Err(error) => return Err(error),
        };
        draft.drop_leftovers(state.compacted_through, &input.plan.segments);
        let mut previous = draft
            .active()
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
            let existing = draft.get(&SegmentSummary::id_for(0, *span)).cloned();
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
                    draft.put(summary.clone());
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
        let merged = self
            .merge_until_small(&mut draft, window, input.model.clone(), input.now)
            .await?;

        let tail: Vec<HistoryItem> = live
            .iter()
            .filter(|item| item.seq >= input.plan.tail_from)
            .map(|item| (*item).clone())
            .collect();
        let latest = draft.active().into_iter().last();
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
        let facts = match self
            .check_preparation(
                revision,
                prepare_facts(
                    &self.stores.facts,
                    self.llm.as_ref(),
                    mined.candidates,
                    tz,
                    input.now,
                    false,
                ),
            )
            .await
        {
            Ok(facts) => facts,
            Err(error) if error.is::<StateConflict>() => {
                return Ok(CompactionOutcome::stale(self.stores.state.load().await?));
            }
            Err(error) => return Err(error),
        };
        let changes = draft.prepare(&self.stores.segments).await?;
        let now = input.now;
        let commit_now = now + Duration::from_std(started.elapsed())?;
        match self
            .commit(revision, commit_now, move |stores, _| async move {
                changes.apply(&stores.segments).await?;
                let report = apply_prepared_facts(&stores.facts, facts, now).await?;
                stores.history.index(&evicted).await?;
                Ok(((segments, merged, report, mined.rejected), state))
            })
            .await?
        {
            CommitOutcome::Applied((segments, merged, facts, rejected), state) => {
                Ok(CompactionOutcome {
                    segments,
                    merged,
                    facts,
                    rejected,
                    state,
                    stale: false,
                })
            }
            CommitOutcome::Stale(state) => Ok(CompactionOutcome::stale(state)),
        }
    }

    /// Merges active summaries one level up ([`plan_merge`]) while they exceed their share of H.
    async fn merge_until_small(
        &self,
        draft: &mut SegmentDraft,
        window: u32,
        model: Option<String>,
        now: DateTime<Utc>,
    ) -> Result<Vec<SegmentSummary>> {
        let budget = self.merge_budget(window);
        let mut merged = Vec::new();
        for _ in 0..MAX_MERGES_PER_COMPACTION {
            let active = draft.active();
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
            draft.put(summary.clone());
            draft.set_parent(&ids, Some(&summary.id));
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
        let started = Instant::now();
        let mut state = self.background_snapshot(now).await?;
        let revision = state.revision;
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
        let facts = self
            .check_preparation(
                revision,
                prepare_facts(
                    &self.stores.facts,
                    self.llm.as_ref(),
                    mined.candidates,
                    self.timezone,
                    now,
                    false,
                ),
            )
            .await?;
        let commit_now = now + Duration::from_std(started.elapsed())?;
        let ((report, rejected), _) = self
            .commit(revision, commit_now, move |stores, _| async move {
                let report = apply_prepared_facts(&stores.facts, facts, now).await?;
                Ok(((report, mined.rejected), state))
            })
            .await?
            .applied(revision)?;
        Ok((report, rejected))
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
                sort: Default::default(),
                geojson: None,
                radius_m: None,
                area: None,
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::MemoryStores;
    use crate::testing::{HashEmbedder, StubLlm};
    use crate::tools::{
        HistoryReadArgs, MemoryForgetArgs, MemorySaveArgs, MemorySearchArgs, WriteGate,
        history_read, memory_forget, memory_save, memory_search,
    };

    fn send<T: Send>(_: &T) {}

    #[tokio::test]
    async fn explicit_write_wait_is_bounded_and_does_not_steal_a_live_lease() {
        let dir = tempfile::tempdir().unwrap();
        let stores = MemoryStores::open_local(dir.path(), Arc::new(HashEmbedder::new("hash", 16)))
            .await
            .unwrap();
        let now = Utc::now();
        let held = stores
            .state
            .save(&CompactionState {
                lease: Some(Lease {
                    holder: "background-engine".into(),
                    until: now + Duration::minutes(10),
                }),
                ..Default::default()
            })
            .await
            .unwrap();
        let engine = MemoryEngine::new(
            stores,
            Arc::new(StubLlm::new()),
            CompactionConfig::default(),
            Tz::UTC,
        );
        let wait = WaitDuration::from_millis(50);
        let started = Instant::now();
        let result = flow_like_types::tokio::time::timeout(
            WaitDuration::from_secs(5),
            engine.commit_with_wait(held.revision, now, wait, |_, state| async move {
                Ok(((), state))
            }),
        )
        .await
        .expect("lease acquisition must stop waiting");
        assert!(result.is_err_and(|error| error.is::<MemoryBusy>()));
        assert!(started.elapsed() >= wait);
        assert_eq!(engine.stores().state.load().await.unwrap(), held);
    }

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
            geojson: None,
            time: None,
            ttl_seconds: None,

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
            timeframe: None,
            sort: Default::default(),
            geojson: None,
            radius_m: None,
            area: None,

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

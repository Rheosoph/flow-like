use std::collections::{BTreeSet, HashMap};

use chrono::{DateTime, Duration, Utc};
use flow_like_types::Result;
use flow_like_types::tokio::time::Instant;

use super::draft::SegmentDraft;
use super::{DeletionReport, MemoryEngine};
use crate::facts::{Evidence, Fact, evidence_trust};
use crate::model::{FactSource, Role, Seq, SeqSpan};
use crate::summary::{
    SegmentSummary, SpanSummaryInput, consolidate, merge_summaries, summarize_span,
};
use crate::tools::{HistoryReader, forget_facts};

struct EvidenceChanges {
    updated: Vec<Fact>,
    deleted: Vec<Fact>,
    remine: Vec<Seq>,
}

impl MemoryEngine {
    /// Follows deleted messages and records the position checked by the deletion sweep.
    /// History reads and summary models finish before the revision-validated commit.
    pub async fn follow_deletions(
        &self,
        seqs: &[Seq],
        through: Option<Seq>,
        reader: &dyn HistoryReader,
        history_window: u32,
        now: DateTime<Utc>,
    ) -> Result<DeletionReport> {
        let started = Instant::now();
        let mut state = self.background_snapshot(now).await?;
        let expected = state.revision;
        let deleted: BTreeSet<Seq> = seqs.iter().copied().collect();
        let mut report = DeletionReport::default();
        let evidence = self
            .check_preparation(
                expected,
                self.prepare_evidence_removal(&deleted, reader, &mut report),
            )
            .await?;
        for seq in &evidence.remine {
            state.unmined.push(SeqSpan::new(*seq, *seq));
        }
        let mut draft = self
            .check_preparation(expected, SegmentDraft::load(&self.stores.segments))
            .await?;
        for summary in draft.covering(&deleted) {
            if summary.level == 0 {
                self.rewrite_level_zero(
                    &mut draft,
                    &summary,
                    &deleted,
                    reader,
                    history_window,
                    now,
                    &mut report,
                )
                .await?;
            } else {
                self.remerge(&mut draft, &summary, history_window, now, &mut report)
                    .await?;
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
        let segments = draft.prepare(&self.stores.segments).await?;
        let seqs = seqs.to_vec();
        let commit_now = now + Duration::from_std(started.elapsed())?;
        let (report, _) = self
            .commit_deletion(expected, commit_now, move |stores, _claimed| async move {
                for fact in &evidence.updated {
                    stores.facts.update_metadata(fact).await?;
                }
                forget_facts(&stores, &evidence.deleted, true).await?;
                stores.history.remove(&seqs).await?;
                segments.apply(&stores.segments).await?;
                Ok((report, state))
            })
            .await?
            .applied(expected)?;
        Ok(report)
    }

    /// Plans evidence removal and recalculates trust from the surviving history.
    async fn prepare_evidence_removal(
        &self,
        deleted: &BTreeSet<Seq>,
        reader: &dyn HistoryReader,
        report: &mut DeletionReport,
    ) -> Result<EvidenceChanges> {
        let seqs: Vec<Seq> = deleted.iter().copied().collect();
        let mut roles: HashMap<Seq, Option<Role>> = HashMap::new();
        let mut gone = Vec::new();
        let mut updated = Vec::new();
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
                report.facts_updated.push(remaining.id.clone());
                updated.push(remaining);
            }
        }
        Ok(EvidenceChanges {
            updated,
            deleted: gone,
            remine: remine.into_iter().collect(),
        })
    }

    /// The most trust any surviving evidence carries, based on its history role.
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

    /// Rewrites a level-zero summary from surviving history; an empty range removes it.
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
        draft: &mut SegmentDraft,
        summary: &SegmentSummary,
        deleted: &BTreeSet<Seq>,
        reader: &dyn HistoryReader,
        window: u32,
        now: DateTime<Utc>,
        report: &mut DeletionReport,
    ) -> Result<()> {
        match self.summarize_again(summary, reader, window, now).await {
            Ok(Some(rewritten)) if rewritten.missing.is_empty() => {
                report.segments_rewritten.push(rewritten.id.clone());
                draft.put(rewritten);
            }
            Ok(None) => {
                draft.delete(&summary.id);
                report.segments_deleted.push(summary.id.clone());
            }
            outcome => {
                if let Err(error) = outcome {
                    tracing::warn!(
                        "Writing summary {} again after a deletion failed; it is cleaned now and rewritten later: {error:#}",
                        summary.id
                    );
                }
                draft.put(summary.scrubbed(deleted));
                report.segments_scrubbed.push(summary.id.clone());
            }
        }
        Ok(())
    }

    /// Retries missing ranges; a range whose model still fails remains queued.
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
        draft: &mut SegmentDraft,
        summary: &SegmentSummary,
        window: u32,
        now: DateTime<Utc>,
        report: &mut DeletionReport,
    ) -> Result<()> {
        let children = draft.children(&summary.id);
        if children.is_empty() {
            draft.delete(&summary.id);
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
        report.segments_rewritten.push(rewritten.id.clone());
        draft.put(rewritten);
        Ok(())
    }

    /// Retries missing or scrubbed summaries and rebuilds their ancestors from the draft.
    pub async fn rewrite_pending(
        &self,
        reader: &dyn HistoryReader,
        history_window: u32,
        now: DateTime<Utc>,
    ) -> Result<DeletionReport> {
        let started = Instant::now();
        let mut state = self.background_snapshot(now).await?;
        let expected = state.revision;
        let mut draft = self
            .check_preparation(expected, SegmentDraft::load(&self.stores.segments))
            .await?;
        let mut report = DeletionReport::default();
        let mut waiting = Vec::new();
        for id in std::mem::take(&mut state.pending_rewrites) {
            let Some(summary) = draft.get(&id).cloned() else {
                continue;
            };
            let filling = !summary.missing.is_empty();
            let outcome = if filling {
                self.fill_missing(&summary, reader, history_window, now)
                    .await
                    .map(Some)
            } else {
                self.summarize_again(&summary, reader, history_window, now)
                    .await
            };
            match outcome {
                Ok(Some(partial)) if !partial.missing.is_empty() => {
                    if filling {
                        draft.put(partial);
                    }
                    waiting.push(id);
                }
                Ok(Some(rewritten)) => {
                    draft.put(rewritten);
                    report.segments_rewritten.push(id);
                    let mut above = summary.parent.clone();
                    while let Some(parent) = above {
                        let Some(parent) = draft.get(&parent).cloned() else {
                            break;
                        };
                        self.remerge(&mut draft, &parent, history_window, now, &mut report)
                            .await?;
                        above = parent.parent;
                    }
                }
                Ok(None) => {
                    draft.delete(&id);
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
        let segments = draft.prepare(&self.stores.segments).await?;
        let commit_now = now + Duration::from_std(started.elapsed())?;
        let (report, _) = self
            .commit(expected, commit_now, move |stores, _claimed| async move {
                segments.apply(&stores.segments).await?;
                Ok((report, state))
            })
            .await?
            .applied(expected)?;
        Ok(report)
    }
}

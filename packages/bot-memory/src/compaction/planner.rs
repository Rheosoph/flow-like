//! The per-turn decisions, without any model call: which tool results the model sees as
//! placeholders, whether compaction is due, where to cut, what stays verbatim and how the
//! evicted span splits into segments.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use chrono_tz::Tz;
use serde::{Deserialize, Serialize};

use super::config::CompactionConfig;
use crate::model::{HistoryItem, Seq, SeqSpan, ToolPart, ToolResult};
use crate::pointer::Pointer;
use crate::summary::render_span_item;
use crate::text::{estimate_tokens, format_tokens, last_chars, one_line, truncate_chars};

/// When the planner runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnPhase {
    /// Right before a model call: only the hard trigger compacts here, synchronously.
    BeforeCall,
    /// After a turn: the soft trigger compacts in the background.
    AfterTurn,
    /// The conversation is idle.
    Idle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Trigger {
    Hard,
    Soft,
    Idle,
}

pub struct PlannerInput<'a> {
    /// The live window in seq order: items after `compacted_through` plus items retained from
    /// earlier compactions. Tombstones are skipped.
    pub items: &'a [HistoryItem],
    pub compacted_through: Option<Seq>,
    /// H, see [`super::history_window`].
    pub history_window: u32,
    /// Tokens of the active segment summaries.
    pub summary_tokens: u32,
    pub now_tokens: u32,
    /// The user messages of the open task, from structured state.
    pub open_task_seqs: &'a [Seq],
    pub phase: TurnPhase,
}

/// A tool result the model sees as a placeholder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MaskedTool {
    pub seq: Seq,
    pub index: u16,
    pub placeholder: String,
    pub placeholder_tokens: u32,
    pub result_tokens: u32,
}

/// The placeholders of one turn.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Masking {
    tools: HashMap<(Seq, u16), MaskedTool>,
}

impl Masking {
    pub fn get(&self, seq: Seq, index: u16) -> Option<&MaskedTool> {
        self.tools.get(&(seq, index))
    }

    pub fn len(&self) -> usize {
        self.tools.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &MaskedTool> {
        self.tools.values()
    }

    /// Tokens of `item` as the model sees it this turn.
    pub fn item_tokens(&self, item: &HistoryItem) -> u32 {
        let tools: u32 = item
            .tools
            .iter()
            .map(|part| {
                part.call_tokens
                    + match (self.get(item.seq, part.index), &part.result) {
                        (Some(masked), _) => masked.placeholder_tokens,
                        (None, Some(result)) => result.tokens,
                        (None, None) => 0,
                    }
            })
            .sum();
        let ui: u32 = item.ui.iter().map(|part| part.tokens).sum();
        item.text_tokens + tools + ui
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TurnPlan {
    pub masking: Masking,
    /// Summaries, the Now block and the live items after masking.
    pub used_tokens: u32,
    pub trigger: Option<Trigger>,
    /// `None` while no compaction is due, or when no cut is possible (one turn fills the window).
    pub compaction: Option<CompactionPlan>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CompactionPlan {
    pub trigger: Trigger,
    /// The items to summarise and mine: everything new before the cut.
    pub evict: SeqSpan,
    /// `evict` split at idle gaps, "New topic" and where one summary call would not read it
    /// whole; one level-0 summary each.
    pub segments: Vec<SeqSpan>,
    /// The first item of the verbatim tail; the cut falls right before it.
    pub tail_from: Seq,
    pub tail_turns: usize,
    /// Items before the cut that stay verbatim (also covered by a summary).
    pub retained: Vec<Seq>,
    pub tokens_before: u32,
    pub projected_tokens: u32,
    pub target_tokens: u32,
    /// The revision of the state the plan was made from. `MemoryEngine::plan` sets it; for a
    /// plan from [`plan_compaction`] set it to the state's `revision`.
    #[serde(default)]
    pub base_revision: u64,
}

impl CompactionPlan {
    pub fn meets_target(&self) -> bool {
        self.projected_tokens <= self.target_tokens
    }

    /// The new `compacted_through`: every position before the tail.
    pub fn compacted_through(&self) -> Seq {
        self.tail_from.saturating_sub(1)
    }
}

/// `[#4812.t1 · weather_api · 2.1k tok · gist]`; a failed result also shows how its output
/// starts and ends: `[#4812.t1 · deploy · failed · 12k tok · exit 1 · starts: … · ends: …]`.
pub fn placeholder(seq: Seq, part: &ToolPart, result: &ToolResult, gist_chars: usize) -> String {
    let pointer = Pointer::Part {
        seq,
        part: part.part(),
    };
    let mut fields = vec![pointer.to_string(), part.name.clone()];
    if result.failed {
        fields.push("failed".to_string());
    }
    fields.push(format_tokens(result.tokens));
    let line = |text: Option<&str>| text.map(one_line).filter(|text| !text.is_empty());
    if let Some(gist) = line(result.gist.as_deref()) {
        fields.push(truncate_chars(&gist, gist_chars));
    }
    if result.failed {
        if let Some(head) = line(result.head.as_deref()) {
            fields.push(format!("starts: {}", truncate_chars(&head, gist_chars)));
        }
        if let Some(tail) = line(result.tail.as_deref()) {
            fields.push(format!("ends: {}", last_chars(&tail, gist_chars)));
        }
    }
    format!("[{}]", fields.join(" · "))
}

fn live_items(items: &[HistoryItem]) -> Vec<&HistoryItem> {
    let mut live: Vec<&HistoryItem> = items.iter().filter(|item| !item.deleted).collect();
    live.sort_by_key(|item| item.seq);
    live.dedup_by_key(|item| item.seq);
    live
}

fn mask_live(live: &[&HistoryItem], window: u32, config: &CompactionConfig) -> Masking {
    let results: Vec<(usize, &HistoryItem, &ToolPart, &ToolResult)> = live
        .iter()
        .enumerate()
        .flat_map(|(index, &item)| {
            item.tools.iter().filter_map(move |part| {
                part.result
                    .as_ref()
                    .map(|result| (index, item, part, result))
            })
        })
        .collect();
    let keep_from = results.len().saturating_sub(config.keep_tool_results);
    let current_turn = live
        .iter()
        .rposition(|item| item.starts_turn())
        .unwrap_or(0);
    let large = config.large_result_limit(window);
    let mut masking = Masking::default();
    for (position, &(index, item, part, result)) in results.iter().enumerate() {
        let oversized = index < current_turn && result.tokens > large;
        let capped = oversized || (position < keep_from && !result.failed);
        if !capped || item.pinned {
            continue;
        }
        let placeholder = placeholder(item.seq, part, result, config.gist_chars);
        let placeholder_tokens = estimate_tokens(&placeholder);
        if placeholder_tokens >= result.tokens {
            continue;
        }
        masking.tools.insert(
            (item.seq, part.index),
            MaskedTool {
                seq: item.seq,
                index: part.index,
                placeholder,
                placeholder_tokens,
                result_tokens: result.tokens,
            },
        );
    }
    masking
}

/// Every tool result older than the last `keep_tool_results` becomes a placeholder, and so does
/// one of an earlier turn above `large_result_share` of H (`window`), so a huge result cannot
/// fill every later prompt. Failed results are evidence and stay verbatim unless they are that
/// large; then their placeholder shows how the output starts and ends. Results in pinned
/// messages always stay.
pub fn mask_tool_results(items: &[HistoryItem], window: u32, config: &CompactionConfig) -> Masking {
    mask_live(&live_items(items), window, config)
}

pub fn trigger_for(
    used: u32,
    window: u32,
    phase: TurnPhase,
    config: &CompactionConfig,
) -> Option<Trigger> {
    match phase {
        TurnPhase::BeforeCall => (used >= config.hard_limit(window)).then_some(Trigger::Hard),
        TurnPhase::AfterTurn => (used >= config.soft_limit(window)).then_some(Trigger::Soft),
        TurnPhase::Idle => (used >= config.idle_limit(window)).then_some(Trigger::Idle),
    }
}

/// A call and the item its result arrived in, as indexes into the live items.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ToolPair {
    pub(crate) call: usize,
    pub(crate) result: Option<usize>,
}

/// `kept` with the other half of every tool pair it holds.
pub(crate) fn close_pairs(mut kept: BTreeSet<usize>, pairs: &[ToolPair]) -> BTreeSet<usize> {
    loop {
        let missing: Vec<usize> = pairs
            .iter()
            .filter_map(|pair| {
                let result = pair.result?;
                match (kept.contains(&pair.call), kept.contains(&result)) {
                    (true, false) => Some(result),
                    (false, true) => Some(pair.call),
                    _ => None,
                }
            })
            .collect();
        if missing.is_empty() {
            return kept;
        }
        kept.extend(missing);
    }
}

pub(crate) fn tool_pairs(live: &[&HistoryItem]) -> Vec<ToolPair> {
    let mut pairs: Vec<ToolPair> = Vec::new();
    let mut by_call: HashMap<&str, usize> = HashMap::new();
    for (index, &item) in live.iter().enumerate() {
        for part in &item.tools {
            match by_call.get(part.call_id.as_str()) {
                None => {
                    by_call.insert(part.call_id.as_str(), pairs.len());
                    pairs.push(ToolPair {
                        call: index,
                        result: part.result.is_some().then_some(index),
                    });
                }
                Some(&pair) => {
                    if part.result.is_some() && pairs[pair].result.is_none() {
                        pairs[pair].result = Some(index);
                    }
                }
            }
        }
    }
    pairs
}

/// A cut right before `index` must not separate a call from its result. Calls in items
/// retained from earlier compactions stay verbatim, so their pairs cannot be split.
fn cut_keeps_pairs(index: usize, pairs: &[ToolPair], first_new: usize) -> bool {
    !pairs.iter().any(|pair| {
        pair.call >= first_new
            && pair
                .result
                .is_some_and(|result| pair.call < index && index <= result)
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Keep {
    Required,
    Failure,
}

struct Window<'a> {
    live: Vec<&'a HistoryItem>,
    tokens: Vec<u32>,
    /// Each item's size in a summary or extraction call.
    rendered: Vec<u32>,
    pairs: Vec<ToolPair>,
    first_new: usize,
}

impl Window<'_> {
    fn tokens_in(&self, range: std::ops::Range<usize>) -> u32 {
        self.tokens[range].iter().sum()
    }

    /// `index` and every item before `cut` it shares a tool pair with, directly or not.
    fn group(&self, index: usize, cut: usize) -> BTreeSet<usize> {
        close_pairs(BTreeSet::from([index]), &self.pairs)
            .into_iter()
            .filter(|&member| member < cut)
            .collect()
    }

    fn turns_from(&self, cut: usize) -> usize {
        self.live[cut..]
            .iter()
            .filter(|item| item.starts_turn())
            .count()
    }

    /// Whether a cut may fall right before `index`: a turn or a topic marker begins there and
    /// no new tool pair is split.
    fn opens_at(&self, index: usize) -> bool {
        self.live[index].opens_span() && cut_keeps_pairs(index, &self.pairs, self.first_new)
    }

    fn cuts(&self) -> Vec<usize> {
        (self.first_new + 1..self.live.len())
            .filter(|&index| self.opens_at(index))
            .collect()
    }

    /// The items before `cut` that stay verbatim, each with the other half of its tool pairs:
    /// a call never stays without its result or the other way round. A group is required when
    /// one of its items is.
    fn retained(&self, cut: usize, open_tasks: &BTreeSet<Seq>) -> Vec<(usize, Keep)> {
        let required_calls: BTreeSet<usize> = self
            .pairs
            .iter()
            .filter(|pair| pair.call < cut && pair.result.is_none_or(|result| result >= cut))
            .map(|pair| pair.call)
            .collect();
        let mut kept: BTreeMap<usize, Keep> = BTreeMap::new();
        for index in 0..cut {
            let item = self.live[index];
            let keep =
                if item.pinned || open_tasks.contains(&item.seq) || required_calls.contains(&index)
                {
                    Keep::Required
                } else if index >= self.first_new && item.has_failure() {
                    Keep::Failure
                } else {
                    continue;
                };
            let group = self.group(index, cut);
            let keep = if group
                .iter()
                .any(|member| kept.get(member) == Some(&Keep::Required))
            {
                Keep::Required
            } else {
                keep
            };
            for member in group {
                kept.insert(member, keep);
            }
        }
        kept.into_iter().collect()
    }

    /// `evict` split into segments: at every "New topic" marker, at idle gaps, and before a turn
    /// that would make one segment longer than a summary call reads.
    fn segments(&self, cut: usize, config: &CompactionConfig) -> Vec<SeqSpan> {
        let mut bounds: Vec<usize> = (self.first_new + 1..cut)
            .filter(|&index| self.opens_at(index) || self.live[index].starts_topic())
            .collect();
        bounds.push(cut);
        let mut starts = vec![self.first_new];
        let mut size = 0;
        let mut from = self.first_new;
        for to in bounds {
            let unit: u32 = self.rendered[from..to].iter().sum();
            if from > self.first_new {
                let item = self.live[from];
                let idle = item.created_at - self.live[from - 1].created_at;
                if item.starts_topic()
                    || idle > config.segment_idle_gap()
                    || size + unit > config.chunk_tokens
                {
                    starts.push(from);
                    size = 0;
                }
            }
            size += unit;
            from = to;
        }
        starts
            .iter()
            .enumerate()
            .map(|(position, &start)| {
                let end = starts.get(position + 1).copied().unwrap_or(cut) - 1;
                SeqSpan::new(self.live[start].seq, self.live[end].seq)
            })
            .collect()
    }
}

fn plan_window(
    window: &Window<'_>,
    input: &PlannerInput<'_>,
    config: &CompactionConfig,
    trigger: Trigger,
) -> Option<CompactionPlan> {
    let cuts = window.cuts();
    let last_cut = *cuts.last()?;
    let history = input.history_window;
    let tail_budget = config.tail_budget(history);
    let mut choice = cuts
        .iter()
        .position(|&cut| {
            window.turns_from(cut) <= config.tail_turns_max
                && window.tokens_in(cut..window.live.len()) <= tail_budget
        })
        .unwrap_or(cuts.len() - 1);

    let open_tasks: BTreeSet<Seq> = input.open_task_seqs.iter().copied().collect();
    let target = config.target_tokens(history);
    let summary_budget = config.segment_summary_budget(history);
    let projected = |cut: usize, retained: &[(usize, Keep)]| -> u32 {
        let segments = window.segments(cut, config).len() as u32;
        let new_summaries =
            (summary_budget * segments).min(window.tokens_in(window.first_new..cut));
        let retained_tokens: u32 = retained
            .iter()
            .map(|&(index, _)| window.tokens[index])
            .sum();
        input.summary_tokens
            + new_summaries
            + config.now_budget(history)
            + retained_tokens
            + window.tokens_in(cut..window.live.len())
    };

    // Over target: release retained failures (oldest first, each with its tool pairs) before
    // shortening the tail.
    let (cut, retained) = loop {
        let cut = cuts[choice];
        let mut retained = window.retained(cut, &open_tasks);
        while projected(cut, &retained) > target {
            let Some(&(oldest, _)) = retained.iter().find(|&&(_, keep)| keep == Keep::Failure)
            else {
                break;
            };
            let group = window.group(oldest, cut);
            retained.retain(|(index, _)| !group.contains(index));
        }
        if projected(cut, &retained) <= target || cut == last_cut {
            break (cut, retained);
        }
        choice += 1;
    };

    Some(CompactionPlan {
        trigger,
        evict: SeqSpan::new(window.live[window.first_new].seq, window.live[cut - 1].seq),
        segments: window.segments(cut, config),
        tail_from: window.live[cut].seq,
        tail_turns: window.turns_from(cut),
        retained: retained
            .iter()
            .map(|&(index, _)| window.live[index].seq)
            .collect(),
        tokens_before: input.summary_tokens + input.now_tokens + window.tokens.iter().sum::<u32>(),
        projected_tokens: projected(cut, &retained),
        target_tokens: target,
        base_revision: 0,
    })
}

fn window<'a>(
    input: &PlannerInput<'a>,
    masking: &Masking,
    config: &CompactionConfig,
) -> Option<Window<'a>> {
    let live = live_items(input.items);
    let first_new = live.iter().position(|item| {
        input
            .compacted_through
            .is_none_or(|through| item.seq > through)
    })?;
    let item_budget = (config.chunk_tokens / 2).max(64);
    Some(Window {
        tokens: live.iter().map(|item| masking.item_tokens(item)).collect(),
        rendered: live
            .iter()
            .map(|item| estimate_tokens(&render_span_item(item, Tz::UTC, item_budget)) + 1)
            .collect(),
        pairs: tool_pairs(&live),
        live,
        first_new,
    })
}

/// Plans a compaction now, whatever the usage: what a "compact now" action or the engine after
/// a trigger runs.
pub fn plan_compaction(
    input: &PlannerInput<'_>,
    config: &CompactionConfig,
    trigger: Trigger,
) -> Option<CompactionPlan> {
    let masking = mask_tool_results(input.items, input.history_window, config);
    plan_window(&window(input, &masking, config)?, input, config, trigger)
}

/// Masks old tool results, measures the turn and plans a compaction when one is due.
pub fn plan_turn(input: &PlannerInput<'_>, config: &CompactionConfig) -> TurnPlan {
    let masking = mask_tool_results(input.items, input.history_window, config);
    let live_tokens: u32 = live_items(input.items)
        .iter()
        .map(|item| masking.item_tokens(item))
        .sum();
    let used_tokens = input.summary_tokens + input.now_tokens + live_tokens;
    let trigger = trigger_for(used_tokens, input.history_window, input.phase, config);
    let compaction = trigger
        .and_then(|trigger| plan_window(&window(input, &masking, config)?, input, config, trigger));
    TurnPlan {
        masking,
        used_tokens,
        trigger,
        compaction,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Role, ToolPart};
    use chrono::{DateTime, Duration, Utc};

    fn at(minutes: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(1_800_000_000, 0).unwrap() + Duration::minutes(minutes)
    }

    fn item(seq: Seq, role: Role, tokens: u32) -> HistoryItem {
        let mut item = HistoryItem::new(seq, format!("m{seq}"), role, at(seq as i64), "x");
        item.text_tokens = tokens;
        item
    }

    /// `turns` user/bot exchanges of `tokens` each, starting at seq 1.
    fn conversation(turns: u64, tokens: u32) -> Vec<HistoryItem> {
        (0..turns)
            .flat_map(|turn| {
                [
                    item(turn * 2 + 1, Role::User, tokens),
                    item(turn * 2 + 2, Role::Assistant, tokens),
                ]
            })
            .collect()
    }

    fn input<'a>(items: &'a [HistoryItem], window: u32, phase: TurnPhase) -> PlannerInput<'a> {
        PlannerInput {
            items,
            compacted_through: None,
            history_window: window,
            summary_tokens: 0,
            now_tokens: 0,
            open_task_seqs: &[],
            phase,
        }
    }

    fn tool_item(seq: Seq, call: &str, result_tokens: Option<u32>) -> HistoryItem {
        let part = ToolPart::call(1, call, "weather_api", 10);
        let part = match result_tokens {
            Some(tokens) => part.with_result(tokens, false, Some("14°C, light rain in Berlin")),
            None => part,
        };
        item(seq, Role::Assistant, 10).with_tool(part)
    }

    #[test]
    fn only_the_last_three_tool_results_stay_in_full() {
        let items: Vec<HistoryItem> = (1..=5)
            .map(|seq| tool_item(seq, &format!("c{seq}"), Some(2_100)))
            .collect();
        let masking = mask_tool_results(&items, 100_000, &CompactionConfig::default());
        let mut masked: Vec<Seq> = masking.iter().map(|tool| tool.seq).collect();
        masked.sort();
        assert_eq!(masked, [1, 2]);
        assert_eq!(
            masking.get(1, 1).unwrap().placeholder,
            "[#1.t1 · weather_api · 2.1k tok · 14°C, light rain in Berlin]"
        );
        assert_eq!(
            masking.item_tokens(&items[0]),
            10 + 10 + masking.get(1, 1).unwrap().placeholder_tokens
        );
        assert_eq!(masking.item_tokens(&items[4]), 10 + 10 + 2_100);
    }

    #[test]
    fn failed_pinned_and_tiny_results_are_never_masked() {
        let failed = item(1, Role::Assistant, 10).with_tool(
            ToolPart::call(1, "c1", "deploy", 10).with_result(900, true, Some("exit 1")),
        );
        let pinned = tool_item(2, "c2", Some(900)).with_pinned(true);
        let tiny = tool_item(3, "c3", Some(2));
        let mut items = vec![failed, pinned, tiny];
        items.extend((4..=6).map(|seq| tool_item(seq, &format!("c{seq}"), Some(900))));
        assert!(mask_tool_results(&items, 100_000, &CompactionConfig::default()).is_empty());
    }

    #[test]
    fn triggers_follow_the_phase() {
        let config = CompactionConfig::default();
        assert_eq!(
            trigger_for(849, 1_000, TurnPhase::BeforeCall, &config),
            None
        );
        assert_eq!(
            trigger_for(850, 1_000, TurnPhase::BeforeCall, &config),
            Some(Trigger::Hard)
        );
        assert_eq!(trigger_for(649, 1_000, TurnPhase::AfterTurn, &config), None);
        assert_eq!(
            trigger_for(900, 1_000, TurnPhase::AfterTurn, &config),
            Some(Trigger::Soft)
        );
        assert_eq!(trigger_for(399, 1_000, TurnPhase::Idle, &config), None);
        assert_eq!(
            trigger_for(400, 1_000, TurnPhase::Idle, &config),
            Some(Trigger::Idle)
        );
    }

    #[test]
    fn a_full_window_cuts_at_a_user_turn_and_keeps_a_bounded_tail() {
        let items = conversation(20, 250);
        let plan = plan_turn(
            &input(&items, 10_000, TurnPhase::BeforeCall),
            &CompactionConfig::default(),
        );
        assert_eq!(plan.used_tokens, 10_000);
        assert_eq!(plan.trigger, Some(Trigger::Hard));
        let compaction = plan.compaction.unwrap();
        let tail_start = items
            .iter()
            .position(|item| item.seq == compaction.tail_from)
            .unwrap();
        assert!(items[tail_start].starts_turn());
        assert_eq!(compaction.tail_turns, 5);
        assert_eq!(compaction.evict, SeqSpan::new(1, compaction.tail_from - 1));
        assert!(
            items[tail_start..]
                .iter()
                .map(|item| item.text_tokens)
                .sum::<u32>()
                <= 2_500
        );
        assert!(compaction.meets_target(), "{compaction:?}");
        assert_eq!(compaction.compacted_through(), compaction.tail_from - 1);
    }

    #[test]
    fn the_tail_keeps_at_most_six_turns() {
        let items = conversation(30, 2);
        let plan = plan_compaction(
            &input(&items, 1_000, TurnPhase::AfterTurn),
            &CompactionConfig::default(),
            Trigger::Soft,
        )
        .unwrap();
        assert_eq!(plan.tail_turns, 6);
        assert_eq!(plan.tail_from, 49);
    }

    #[test]
    fn the_cut_never_separates_a_call_from_a_late_result() {
        let mut items = conversation(12, 20);
        // The call in turn 8 (seq 16) resolves in turn 10 (seq 20).
        items[15] = tool_item(16, "late", None);
        items[19] = item(20, Role::Assistant, 10)
            .with_tool(ToolPart::call(1, "late", "render_video", 0).with_result(40, false, None));
        let plan = plan_compaction(
            &input(&items, 1_000, TurnPhase::BeforeCall),
            &CompactionConfig {
                tail_turns_max: 3,
                ..CompactionConfig::default()
            },
            Trigger::Hard,
        )
        .unwrap();
        // A tail of three turns would start at seq 19, between the call (16) and its result
        // (20); the cut moves past the result, so the whole pair is summarised together.
        assert!(!(17..=20).contains(&plan.tail_from), "{plan:?}");
        assert_eq!(plan.tail_from, 21);
        assert!(plan.evict.contains(16) && plan.evict.contains(20));
    }

    #[test]
    fn pinned_open_task_unresolved_and_failed_items_stay_verbatim() {
        let mut items = conversation(16, 20);
        items[2] = items[2].clone().with_pinned(true); // seq 3
        items[5] = tool_item(6, "pending", None); // seq 6: unresolved
        items[7] = items[7].clone().with_failed(true); // seq 8
        let open = [9];
        let plan = plan_compaction(
            &PlannerInput {
                open_task_seqs: &open,
                ..input(&items, 2_000, TurnPhase::BeforeCall)
            },
            &CompactionConfig::default(),
            Trigger::Hard,
        )
        .unwrap();
        assert_eq!(plan.retained, [3, 6, 8, 9]);
        assert!(plan.evict.contains(3) && plan.evict.contains(9));
    }

    /// Review finding 6: a call and its result in different items stay or go together.
    #[test]
    fn retention_keeps_both_halves_of_a_tool_pair() {
        let mut items = conversation(16, 20);
        items[1] = tool_item(2, "deploy", None);
        items[2] = item(3, Role::Tool, 5).with_tool(
            ToolPart::call(1, "deploy", "deploy", 0).with_result(40, true, Some("exit 1")),
        );
        items[5] = tool_item(6, "report", None).with_pinned(true);
        items[6] = item(7, Role::Tool, 5).with_tool(
            ToolPart::call(1, "report", "fetch_report", 0).with_result(40, false, Some("rows")),
        );
        let plan = plan_compaction(
            &input(&items, 2_000, TurnPhase::BeforeCall),
            &CompactionConfig::default(),
            Trigger::Hard,
        )
        .unwrap();
        assert!(plan.tail_from > 7, "{plan:?}");
        assert_eq!(plan.retained, [2, 3, 6, 7]);

        let tight = plan_compaction(
            &input(&items, 400, TurnPhase::BeforeCall),
            &CompactionConfig::default(),
            Trigger::Hard,
        )
        .unwrap();
        assert!(
            !tight.retained.contains(&2) && !tight.retained.contains(&3),
            "a released failure takes its call along: {tight:?}"
        );
        assert!(tight.retained.contains(&6) && tight.retained.contains(&7));
    }

    #[test]
    fn retained_failures_are_released_first_when_the_target_is_missed() {
        let mut items = conversation(16, 20);
        items[1].text_tokens = 400;
        items[1].failed = true; // seq 2: a large failed turn
        items[3] = items[3].clone().with_pinned(true); // seq 4
        let plan = plan_compaction(
            &input(&items, 1_000, TurnPhase::BeforeCall),
            &CompactionConfig::default(),
            Trigger::Hard,
        )
        .unwrap();
        assert_eq!(plan.retained, [4]);
        assert!(plan.meets_target(), "{plan:?}");
    }

    #[test]
    fn earlier_retained_items_are_kept_or_released_but_not_summarised_again() {
        let mut items = conversation(16, 20);
        items[1].failed = true; // seq 2, retained by an earlier compaction
        items[2] = items[2].clone().with_pinned(true); // seq 3
        let live: Vec<HistoryItem> = items
            .iter()
            .filter(|item| matches!(item.seq, 2 | 3) || item.seq > 10)
            .cloned()
            .collect();
        let plan = plan_compaction(
            &PlannerInput {
                compacted_through: Some(10),
                ..input(&live, 1_000, TurnPhase::BeforeCall)
            },
            &CompactionConfig::default(),
            Trigger::Hard,
        )
        .unwrap();
        assert_eq!(plan.evict.from, 11);
        assert_eq!(plan.retained, [3]);
    }

    #[test]
    fn idle_gaps_and_new_topics_split_the_evicted_span() {
        let mut items = conversation(16, 20);
        for item in items.iter_mut().filter(|item| item.seq >= 7) {
            item.created_at += Duration::hours(7);
        }
        items[10] = items[10].clone().with_new_topic(true); // seq 11
        let plan = plan_compaction(
            &input(&items, 1_000, TurnPhase::BeforeCall),
            &CompactionConfig::default(),
            Trigger::Hard,
        )
        .unwrap();
        assert_eq!(
            plan.segments,
            [
                SeqSpan::new(1, 6),
                SeqSpan::new(7, 10),
                SeqSpan::new(11, plan.evict.to)
            ]
        );
    }

    /// Review finding 11: each level-0 segment fits one summary call, so no segment's claims
    /// are cut back to one call's budget.
    #[test]
    fn segments_split_so_each_fits_one_summary_call() {
        let items: Vec<HistoryItem> = (1..=80)
            .map(|seq| {
                let role = if seq % 2 == 1 {
                    Role::User
                } else {
                    Role::Assistant
                };
                HistoryItem::new(
                    seq,
                    format!("m{seq}"),
                    role,
                    at(seq as i64),
                    format!("Message {seq}: {}", "words about the garden ".repeat(10)),
                )
            })
            .collect();
        let config = CompactionConfig {
            chunk_tokens: 500,
            ..CompactionConfig::default()
        };
        let plan = plan_compaction(
            &input(&items, 100_000, TurnPhase::AfterTurn),
            &config,
            Trigger::Soft,
        )
        .unwrap();
        assert!(plan.segments.len() > 1, "{plan:?}");
        assert_eq!(plan.segments.first().map(|span| span.from), Some(1));
        assert_eq!(
            plan.segments.last().map(|span| span.to),
            Some(plan.evict.to)
        );
        for (span, next) in plan.segments.iter().zip(plan.segments.iter().skip(1)) {
            assert_eq!(span.to + 1, next.from);
            assert!(items[next.from as usize - 1].starts_turn());
        }
        for span in &plan.segments {
            let lines: Vec<(Seq, String)> = items
                .iter()
                .filter(|item| span.contains(item.seq))
                .map(|item| {
                    (
                        item.seq,
                        crate::summary::render_span_item(
                            item,
                            chrono_tz::Tz::UTC,
                            config.chunk_tokens / 2,
                        ),
                    )
                })
                .collect();
            assert_eq!(
                crate::summary::chunk_lines(lines, config.chunk_tokens).len(),
                1,
                "{span} needs more than one summary call"
            );
        }
    }

    /// Review finding 8: one huge tool result from an earlier turn becomes a placeholder even
    /// among the last three, so it cannot fill every later prompt.
    #[test]
    fn oversized_results_of_earlier_turns_are_masked() {
        let mut items = conversation(4, 20);
        items[1] = tool_item(2, "big", Some(10_000));
        let window = 3_973;
        let plan = plan_turn(
            &input(&items, window, TurnPhase::BeforeCall),
            &CompactionConfig::default(),
        );
        let masked = plan.masking.get(2, 1).expect("the 10k result is masked");
        assert!(
            masked
                .placeholder
                .starts_with("[#2.t1 · weather_api · 10k tok")
        );
        let mut current = conversation(4, 20);
        current[7] = tool_item(8, "big", Some(10_000));
        let plan = plan_turn(
            &input(&current, window, TurnPhase::BeforeCall),
            &CompactionConfig::default(),
        );
        assert!(
            plan.masking.get(8, 1).is_none(),
            "the current turn sees its own result"
        );
    }

    /// Follow-up 2: a failed result stays as evidence, but a huge one from an earlier turn is
    /// capped like any other result.
    #[test]
    fn oversized_failed_results_of_earlier_turns_are_capped() {
        let mut items = conversation(4, 20);
        items[1] = item(2, Role::Assistant, 10).with_tool(
            ToolPart::call(1, "deploy", "deploy", 10)
                .with_result(10_000, true, Some("exit 1"))
                .with_excerpt(
                    "npm ERR! code ELIFECYCLE\nnpm ERR! errno 1",
                    &format!(
                        "{}\nError: Process exited with code 1",
                        "trace line\n".repeat(50)
                    ),
                ),
        );
        items[3] = item(4, Role::Assistant, 10).with_tool(
            ToolPart::call(1, "lint", "lint", 10).with_result(300, true, Some("2 errors")),
        );
        let plan = plan_turn(
            &input(&items, 3_973, TurnPhase::BeforeCall),
            &CompactionConfig::default(),
        );
        let capped = plan.masking.get(2, 1).expect("the 10k failure is capped");
        assert!(
            capped
                .placeholder
                .starts_with("[#2.t1 · deploy · failed · 10k tok · exit 1 · starts: npm ERR! code ELIFECYCLE npm ERR! errno 1 · ends: …"),
            "{}",
            capped.placeholder
        );
        assert!(
            capped
                .placeholder
                .ends_with("Error: Process exited with code 1]"),
            "{}",
            capped.placeholder
        );
        assert!(capped.placeholder_tokens < 100);
        assert!(
            plan.masking.get(4, 1).is_none(),
            "a small failure stays in full"
        );
    }

    /// Follow-up 3: a "New topic" marker starts a segment wherever it stands, also between
    /// turns.
    #[test]
    fn a_topic_marker_always_starts_a_segment() {
        let mut items = conversation(8, 20);
        items.push(item(17, Role::System, 2).with_new_topic(true));
        items.extend(conversation(8, 20).into_iter().map(|mut item| {
            item.seq += 17;
            item.created_at = at(item.seq as i64);
            item
        }));
        let plan = plan_compaction(
            &input(&items, 1_000, TurnPhase::BeforeCall),
            &CompactionConfig::default(),
            Trigger::Hard,
        )
        .unwrap();
        assert!(plan.evict.to > 17, "{plan:?}");
        assert!(
            plan.segments.iter().any(|span| span.from == 17),
            "{:?}",
            plan.segments
        );

        items[16] = HistoryItem::topic(17, "m17", at(17), "Trip planning");
        assert!(items[16].starts_topic() && !items[16].starts_turn());
        let plan = plan_compaction(
            &input(&items, 1_000, TurnPhase::BeforeCall),
            &CompactionConfig::default(),
            Trigger::Hard,
        )
        .unwrap();
        assert!(
            plan.segments.iter().any(|span| span.from == 17),
            "{:?}",
            plan.segments
        );
        let rendered = crate::summary::render_span_item(&items[16], chrono_tz::Tz::UTC, 100);
        assert!(rendered.ends_with("topic] Trip planning"), "{rendered}");
    }

    #[test]
    fn no_compaction_without_a_turn_boundary_to_cut_at() {
        let items = vec![item(1, Role::User, 900), item(2, Role::Assistant, 300)];
        let plan = plan_turn(
            &input(&items, 1_000, TurnPhase::BeforeCall),
            &CompactionConfig::default(),
        );
        assert_eq!(plan.trigger, Some(Trigger::Hard));
        assert_eq!(plan.compaction, None);
    }

    #[test]
    fn tombstones_do_not_count() {
        let mut items = conversation(10, 50);
        for item in items.iter_mut().take(10) {
            item.deleted = true;
        }
        let plan = plan_turn(
            &input(&items, 1_000, TurnPhase::AfterTurn),
            &CompactionConfig::default(),
        );
        assert_eq!(plan.used_tokens, 500);
        assert_eq!(plan.trigger, None);
    }
}

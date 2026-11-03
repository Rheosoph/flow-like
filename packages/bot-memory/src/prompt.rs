//! Prompt assembly in the cache-stable order — system and tools, about-you card, segment
//! summaries with the Now block, state block, verbatim tail, then recalled facts and the user's
//! message — within per-model budgets (research §3.7).

use std::collections::BTreeSet;

use chrono_tz::Tz;
use serde::{Deserialize, Serialize};

use crate::compaction::{Masking, close_pairs, history_window, tool_pairs, usable_window};
use crate::facts::Fact;
use crate::model::{HistoryItem, Seq};
use crate::retrieval::memory_block;
use crate::summary::{NowBlock, SegmentSummary, render_span_item};
use crate::text::{estimate_tokens, truncate_to_tokens};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelProfile {
    Hosted,
    /// An 8k-context llama.cpp model.
    LlamaCpp8k,
    /// A 4k-context MLX model, chat only.
    Mlx4k,
}

/// Token allocations per section. The small-model numbers are the research's starting splits,
/// not proven optima.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelBudget {
    pub profile: ModelProfile,
    pub context_window: u32,
    pub system_tools: u32,
    pub max_tools: Option<usize>,
    pub about_state: u32,
    pub summaries: u32,
    pub facts_tokens: u32,
    pub facts_count: usize,
    pub tail_min: u32,
    pub output: u32,
}

impl ModelBudget {
    /// Large windows give about-you/state at least 1k tokens and facts 1k; small ones a tenth of
    /// W each, so history keeps most of the window.
    pub fn hosted(context_window: u32, max_output: u32) -> Self {
        let usable = usable_window(context_window, max_output, None);
        let floor = (usable / 10).min(1_000);
        Self {
            profile: ModelProfile::Hosted,
            context_window,
            system_tools: usable / 8,
            max_tools: None,
            about_state: (usable / 40).max(floor),
            summaries: usable * 15 / 100,
            facts_tokens: floor,
            facts_count: 6,
            tail_min: usable / 4,
            output: max_output,
        }
    }

    pub fn llama_cpp_8k() -> Self {
        Self {
            profile: ModelProfile::LlamaCpp8k,
            context_window: 8_192,
            system_tools: 1_500,
            max_tools: Some(5),
            about_state: 600,
            summaries: 800,
            facts_tokens: 300,
            facts_count: 3,
            tail_min: 3_000,
            output: 1_000,
        }
    }

    pub fn mlx_4k() -> Self {
        Self {
            profile: ModelProfile::Mlx4k,
            context_window: 4_096,
            system_tools: 600,
            max_tools: Some(1),
            about_state: 300,
            summaries: 400,
            facts_tokens: 150,
            facts_count: 2,
            tail_min: 1_500,
            output: 700,
        }
    }

    /// W for this model.
    pub fn usable_window(&self) -> u32 {
        usable_window(self.context_window, self.output, None)
    }

    /// H for the planner: W minus the system/tools, about-you/state and fact allocations. Hosts
    /// that measured their fixed sections pass those to [`history_window`] instead.
    pub fn history_window(&self) -> u32 {
        history_window(
            self.usable_window(),
            self.system_tools + self.about_state + self.facts_tokens,
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SectionKind {
    SystemAndTools,
    AboutYou,
    Summaries,
    State,
    Tail,
    Memory,
    UserMessage,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PromptSection {
    pub kind: SectionKind,
    pub text: String,
    pub tokens: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct BudgetReport {
    pub total_tokens: u32,
    pub usable_window: u32,
    /// Oldest summaries left out; a merge is due.
    pub dropped_summaries: Vec<String>,
    /// Oldest tail items left out (whole turns).
    pub dropped_tail: Vec<Seq>,
    pub dropped_facts: usize,
    pub truncated: Vec<SectionKind>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AssembledPrompt {
    /// In cache-stable order.
    pub sections: Vec<PromptSection>,
    /// The tail items that fit, oldest first; hosts render them as native messages with the
    /// turn's masking.
    pub tail_seqs: Vec<Seq>,
    pub report: BudgetReport,
}

impl AssembledPrompt {
    pub fn section(&self, kind: SectionKind) -> Option<&PromptSection> {
        self.sections.iter().find(|section| section.kind == kind)
    }
}

pub struct PromptInput<'a> {
    pub system_and_tools: &'a str,
    /// Tokens of tool definitions sent next to the prompt.
    pub tool_tokens: u32,
    pub about_you: &'a str,
    /// Active summaries.
    pub summaries: &'a [SegmentSummary],
    pub now_block: Option<&'a NowBlock>,
    /// Open tasks, approvals, running jobs and wake-ups, rendered by the host from its records.
    pub state_block: &'a str,
    /// The verbatim items in seq order: retained ones, then the tail.
    pub tail: &'a [HistoryItem],
    /// The user messages of the open task: like pinned items and unresolved calls, never left
    /// out.
    pub open_task_seqs: &'a [Seq],
    pub masking: &'a Masking,
    /// Recalled facts, best first.
    pub facts: &'a [Fact],
    pub user_message: &'a str,
    pub timezone: Tz,
}

/// A tail item as text, with masked tool results as their placeholders.
pub fn render_tail_item(item: &HistoryItem, masking: &Masking, tz: Tz) -> String {
    let mut shown = item.clone();
    let mut placeholders = Vec::new();
    shown
        .tools
        .retain(|part| match masking.get(item.seq, part.index) {
            Some(masked) => {
                placeholders.push(masked.placeholder.clone());
                false
            }
            None => true,
        });
    let mut text = render_span_item(&shown, tz, u32::MAX);
    for line in placeholders {
        text.push_str("\n  ");
        text.push_str(&line);
    }
    text
}

fn section(kind: SectionKind, text: String, extra_tokens: u32) -> PromptSection {
    let tokens = estimate_tokens(&text) + extra_tokens;
    PromptSection { kind, text, tokens }
}

/// Lines kept from the top while they fit, then a note of how many were left out.
fn keep_lines(text: &str, budget: u32) -> (String, bool) {
    if estimate_tokens(text) <= budget {
        return (text.to_string(), false);
    }
    let lines: Vec<&str> = text.lines().collect();
    let mut kept: Vec<&str> = Vec::new();
    let mut used = 8;
    for line in &lines {
        let tokens = estimate_tokens(line) + 1;
        if used + tokens > budget {
            break;
        }
        used += tokens;
        kept.push(line);
    }
    let left_out = lines.len() - kept.len();
    (
        format!("{}\n(+{left_out} more lines)", kept.join("\n")),
        true,
    )
}

fn turn_starts(items: &[HistoryItem]) -> Vec<usize> {
    let mut starts: Vec<usize> = items
        .iter()
        .enumerate()
        .filter(|(_, item)| item.starts_turn())
        .map(|(index, _)| index)
        .collect();
    if starts.first() != Some(&0) {
        starts.insert(0, 0);
    }
    starts
}

/// The verbatim items that fit `room`, as indexes: whole turns from the newest back, plus the
/// items that never go (pinned, the open task's messages, unresolved calls), each with the
/// other half of its tool pairs. When nothing fits, the newest turn and those items.
fn fitting_tail(input: &PromptInput<'_>, item_tokens: &[u32], room: u32) -> BTreeSet<usize> {
    let items: Vec<&HistoryItem> = input.tail.iter().collect();
    let pairs = tool_pairs(&items);
    let required: BTreeSet<usize> = items
        .iter()
        .enumerate()
        .filter(|(_, item)| item.pinned || input.open_task_seqs.contains(&item.seq))
        .map(|(index, _)| index)
        .chain(
            pairs
                .iter()
                .filter(|pair| pair.result.is_none())
                .map(|pair| pair.call),
        )
        .collect();
    let mut kept = BTreeSet::new();
    for start in turn_starts(input.tail) {
        kept = close_pairs(
            (start..items.len())
                .chain(required.iter().copied())
                .collect(),
            &pairs,
        );
        if kept.iter().map(|&index| item_tokens[index]).sum::<u32>() <= room {
            break;
        }
    }
    kept
}

/// Assembles the prompt. The state block is never dropped (it is the only record of open work);
/// over its budget it is cut by lines with a note. Summaries go oldest first, the verbatim
/// items by whole turns oldest first, never pinned or open-task items, unresolved calls or half
/// of a tool pair.
pub fn assemble(input: &PromptInput<'_>, budget: &ModelBudget) -> AssembledPrompt {
    let mut report = BudgetReport {
        usable_window: budget.usable_window(),
        ..BudgetReport::default()
    };
    let tz = input.timezone;

    let system = section(
        SectionKind::SystemAndTools,
        input.system_and_tools.to_string(),
        input.tool_tokens,
    );
    if system.tokens > budget.system_tools {
        report.warnings.push(format!(
            "System prompt and tools take {} tokens; the {:?} budget is {}",
            system.tokens, budget.profile, budget.system_tools
        ));
    }

    let state_text = match input.state_block.trim() {
        "" => String::new(),
        block => format!("Current state (from records, authoritative):\n{block}"),
    };
    let (state_text, state_cut) = keep_lines(&state_text, budget.about_state);
    if state_cut {
        report.truncated.push(SectionKind::State);
        report
            .warnings
            .push("The state block is over its budget; some lines were left out".into());
    }
    let state = section(SectionKind::State, state_text, 0);

    let about_budget = budget.about_state.saturating_sub(state.tokens);
    let about_text = match input.about_you.trim() {
        "" => String::new(),
        about => format!("About the user:\n{about}"),
    };
    if estimate_tokens(&about_text) > about_budget {
        report.truncated.push(SectionKind::AboutYou);
    }
    let about_text = if about_budget < estimate_tokens("About the user:\n") + 4 {
        String::new()
    } else {
        truncate_to_tokens(&about_text, about_budget)
    };
    let about = section(SectionKind::AboutYou, about_text, 0);

    let now_text = input
        .now_block
        .map(|block| block.text.clone())
        .unwrap_or_default();
    let mut summary_budget = budget.summaries.saturating_sub(estimate_tokens(&now_text));
    let mut ordered: Vec<&SegmentSummary> = input.summaries.iter().collect();
    ordered.sort_by_key(|summary| summary.span.from);
    let mut kept: Vec<&SegmentSummary> = Vec::new();
    for (position, summary) in ordered.iter().enumerate().rev() {
        let tokens = summary.tokens() + 1;
        if tokens > summary_budget {
            report.dropped_summaries.extend(
                ordered[..=position]
                    .iter()
                    .map(|summary| summary.id.clone()),
            );
            break;
        }
        summary_budget -= tokens;
        kept.push(summary);
    }
    kept.reverse();
    let mut summaries_text = String::new();
    if !kept.is_empty() {
        summaries_text.push_str(
            "Earlier in this conversation (summaries; #seq points into the history; the newest statement wins):",
        );
        for summary in &kept {
            summaries_text.push('\n');
            summaries_text.push_str(&summary.render(tz));
        }
    }
    if !now_text.is_empty() {
        if !summaries_text.is_empty() {
            summaries_text.push_str("\n\n");
        }
        summaries_text.push_str(&now_text);
    }
    let summaries = section(SectionKind::Summaries, summaries_text, 0);

    let shown_facts = &input.facts[..input.facts.len().min(budget.facts_count)];
    let memory_text = memory_block(shown_facts, tz, budget.facts_tokens);
    let shown = memory_text.lines().count().saturating_sub(2);
    report.dropped_facts = input.facts.len() - shown;
    let memory = section(SectionKind::Memory, memory_text, 0);
    let user = section(SectionKind::UserMessage, input.user_message.to_string(), 0);

    let fixed = system.tokens
        + about.tokens
        + summaries.tokens
        + state.tokens
        + memory.tokens
        + user.tokens;
    let room = report.usable_window.saturating_sub(fixed);
    let item_tokens: Vec<u32> = input
        .tail
        .iter()
        .map(|item| input.masking.item_tokens(item))
        .collect();
    let kept = fitting_tail(input, &item_tokens, room);
    let tail_items: Vec<&HistoryItem> = kept.iter().map(|&index| &input.tail[index]).collect();
    report.dropped_tail = input
        .tail
        .iter()
        .enumerate()
        .filter(|(index, _)| !kept.contains(index))
        .map(|(_, item)| item.seq)
        .collect();
    let tail_tokens: u32 = kept.iter().map(|&index| item_tokens[index]).sum();
    if tail_tokens > room {
        report.warnings.push(format!(
            "The newest turn and the items that must stay take {tail_tokens} tokens; {room} are left for them"
        ));
    }
    if tail_tokens < budget.tail_min && !report.dropped_tail.is_empty() {
        report.warnings.push(format!(
            "The verbatim tail has {tail_tokens} tokens, below its minimum of {}",
            budget.tail_min
        ));
    }
    let tail_text: Vec<String> = tail_items
        .iter()
        .map(|item| render_tail_item(item, input.masking, tz))
        .collect();
    let tail = PromptSection {
        kind: SectionKind::Tail,
        text: tail_text.join("\n"),
        tokens: tail_tokens,
    };

    let sections = vec![system, about, summaries, state, tail, memory, user];
    report.total_tokens = sections.iter().map(|section| section.tokens).sum();
    AssembledPrompt {
        tail_seqs: tail_items.iter().map(|item| item.seq).collect(),
        sections,
        report,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compaction::{CompactionConfig, mask_tool_results};
    use crate::model::{Role, SeqSpan, ToolPart};
    use crate::summary::{Claim, SUMMARY_PROMPT_VERSION, Section, SummaryBody};
    use chrono::{DateTime, Duration, Utc};

    fn at(minutes: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(1_800_000_000, 0).unwrap() + Duration::minutes(minutes)
    }

    fn summary(from: Seq, to: Seq, claims: usize) -> SegmentSummary {
        let mut body = SummaryBody::default();
        for n in 0..claims {
            body.push(
                Section::Decisions,
                Claim {
                    text: format!("Decision number {n} about the launch plan"),
                    seqs: vec![from + n as u64],
                    unverified: false,
                },
            );
        }
        let span = SeqSpan::new(from, to);
        SegmentSummary {
            id: SegmentSummary::id_for(0, span),
            level: 0,
            span,
            t_from: at(0),
            t_to: at(1),
            body,
            children: Vec::new(),
            child_spans: Vec::new(),
            user_seqs: Vec::new(),
            missing: Vec::new(),
            parent: None,
            version: 1,
            prompt_version: SUMMARY_PROMPT_VERSION,
            model: None,
            created_at: at(2),
        }
    }

    fn tail(turns: u64, tokens: u32) -> Vec<HistoryItem> {
        (0..turns)
            .flat_map(|turn| {
                let mut user =
                    HistoryItem::new(100 + turn * 2, "u", Role::User, at(turn as i64), "question");
                user.text_tokens = tokens;
                let mut bot = HistoryItem::new(
                    101 + turn * 2,
                    "b",
                    Role::Assistant,
                    at(turn as i64),
                    "answer",
                );
                bot.text_tokens = tokens;
                [user, bot]
            })
            .collect()
    }

    fn input<'a>(
        summaries: &'a [SegmentSummary],
        tail: &'a [HistoryItem],
        masking: &'a Masking,
        facts: &'a [Fact],
        state: &'a str,
    ) -> PromptInput<'a> {
        PromptInput {
            system_and_tools: "You are Atlas, a helpful bot.",
            tool_tokens: 200,
            about_you: "Ada, works on rockets.",
            summaries,
            now_block: None,
            state_block: state,
            tail,
            open_task_seqs: &[],
            masking,
            facts,
            user_message: "What's next?",
            timezone: Tz::UTC,
        }
    }

    #[test]
    fn budgets_follow_the_research_tables() {
        let llama = ModelBudget::llama_cpp_8k();
        assert_eq!(
            (llama.facts_count, llama.facts_tokens, llama.summaries),
            (3, 300, 800)
        );
        assert_eq!(llama.usable_window(), 6_373);
        assert_eq!(llama.history_window(), 6_373 - 2_400);
        let mlx = ModelBudget::mlx_4k();
        assert_eq!(
            (mlx.facts_count, mlx.max_tools, mlx.tail_min),
            (2, Some(1), 1_500)
        );
        let hosted = ModelBudget::hosted(200_000, 16_000);
        assert_eq!((hosted.facts_count, hosted.facts_tokens), (6, 1_000));
        assert_eq!(hosted.usable_window(), 164_000);
        assert!(hosted.history_window() > hosted.usable_window() * 3 / 4);
    }

    #[test]
    fn sections_come_in_cache_stable_order_with_memory_right_before_the_user() {
        let summaries = [summary(1, 50, 2)];
        let items = tail(3, 50);
        let masking = Masking::default();
        let prompt = assemble(
            &input(&summaries, &items, &masking, &[], "- Task: ship v2 (#90)"),
            &ModelBudget::llama_cpp_8k(),
        );
        let kinds: Vec<SectionKind> = prompt.sections.iter().map(|section| section.kind).collect();
        assert_eq!(
            kinds,
            [
                SectionKind::SystemAndTools,
                SectionKind::AboutYou,
                SectionKind::Summaries,
                SectionKind::State,
                SectionKind::Tail,
                SectionKind::Memory,
                SectionKind::UserMessage
            ]
        );
        assert!(
            prompt
                .section(SectionKind::State)
                .unwrap()
                .text
                .contains("- Task: ship v2 (#90)")
        );
        assert_eq!(prompt.tail_seqs.len(), 6);
        assert!(
            prompt.report.warnings.is_empty(),
            "{:?}",
            prompt.report.warnings
        );
        assert_eq!(
            prompt.section(SectionKind::SystemAndTools).unwrap().tokens,
            estimate_tokens("You are Atlas, a helpful bot.") + 200
        );
    }

    #[test]
    fn the_oldest_summaries_and_whole_oldest_turns_give_way_first() {
        let summaries = [
            summary(1, 50, 40),
            summary(51, 100, 40),
            summary(101, 150, 40),
        ];
        let items = tail(12, 300);
        let masking = Masking::default();
        let prompt = assemble(
            &input(&summaries, &items, &masking, &[], "none"),
            &ModelBudget::llama_cpp_8k(),
        );
        assert_eq!(
            prompt.report.dropped_summaries.first().map(String::as_str),
            Some("s0-1-50")
        );
        assert!(prompt.section(SectionKind::Summaries).unwrap().tokens <= 800 + 30);
        assert!(!prompt.report.dropped_tail.is_empty());
        assert!(prompt.report.total_tokens <= prompt.report.usable_window);
        let first_kept = items
            .iter()
            .find(|item| item.seq == prompt.tail_seqs[0])
            .unwrap();
        assert!(first_kept.starts_turn());
    }

    #[test]
    fn the_state_block_is_cut_by_lines_never_dropped() {
        let state: String = (0..200)
            .map(|n| format!("- Open task {n}: follow up with the vendor\n"))
            .collect();
        let masking = Masking::default();
        let prompt = assemble(
            &input(&[], &[], &masking, &[], &state),
            &ModelBudget::mlx_4k(),
        );
        let section = prompt.section(SectionKind::State).unwrap();
        assert!(section.text.contains("- Open task 0:"));
        assert!(section.text.contains("more lines)"));
        assert!(section.tokens <= 300);
        assert!(prompt.report.truncated.contains(&SectionKind::State));
    }

    #[test]
    fn masked_tool_results_render_as_placeholders_in_the_tail() {
        let items: Vec<HistoryItem> = (1..=5)
            .map(|seq| {
                HistoryItem::new(seq, "m", Role::Assistant, at(seq as i64), "checked").with_tool(
                    ToolPart::call(1, format!("c{seq}"), "weather_api", 5).with_result(
                        2_000,
                        false,
                        Some("rain"),
                    ),
                )
            })
            .collect();
        let masking = mask_tool_results(&items, 100_000, &CompactionConfig::default());
        let text = render_tail_item(&items[0], &masking, Tz::UTC);
        assert!(
            text.contains("[#1.t1 · weather_api · 2k tok · rain]"),
            "{text}"
        );
        assert!(!text.contains("[tool weather_api"));
        let full = render_tail_item(&items[4], &masking, Tz::UTC);
        assert!(
            full.contains("#5.t1 [tool weather_api · ok · 2k tok] rain"),
            "{full}"
        );
    }

    fn pairs_complete(items: &[HistoryItem], shown: &[Seq]) -> bool {
        let mut calls: Vec<(&str, Seq, Option<Seq>)> = Vec::new();
        for item in items {
            for tool in &item.tools {
                match calls.iter_mut().find(|(id, _, _)| *id == tool.call_id) {
                    Some(pair) if tool.result.is_some() => pair.2 = Some(item.seq),
                    Some(_) => {}
                    None => calls.push((
                        tool.call_id.as_str(),
                        item.seq,
                        tool.result.as_ref().map(|_| item.seq),
                    )),
                }
            }
        }
        calls.iter().all(|(_, call, result)| {
            result.is_none_or(|result| shown.contains(call) == shown.contains(&result))
        })
    }

    /// Review finding 6: under pressure the oldest tail turns go first; pinned and unresolved
    /// retained items stay, and no call loses its result.
    #[test]
    fn assembly_keeps_retained_items_and_tool_pairs() {
        let mut pinned = HistoryItem::new(3, "p", Role::User, at(0), "Pinned: badge code 7781");
        pinned.pinned = true;
        let pending = HistoryItem::new(6, "c", Role::Assistant, at(0), "Rendering the video.")
            .with_tool(ToolPart::call(1, "render", "render_video", 20));
        let mut items = vec![pinned, pending];
        items.extend(tail(12, 300));
        let masking = Masking::default();
        let prompt = assemble(
            &input(&[], &items, &masking, &[], "none"),
            &ModelBudget::llama_cpp_8k(),
        );
        assert!(!prompt.report.dropped_tail.is_empty());
        assert!(
            prompt.tail_seqs.contains(&3) && prompt.tail_seqs.contains(&6),
            "{:?}",
            prompt.report.dropped_tail
        );

        for turn in 0..11 {
            let mut items = tail(12, 300);
            let call = (turn * 2 + 1) as usize;
            items[call] =
                items[call]
                    .clone()
                    .with_tool(ToolPart::call(1, "late", "render_video", 10));
            items[call + 2] = items[call + 2].clone().with_tool(
                ToolPart::call(1, "late", "render_video", 0).with_result(40, false, None),
            );
            let prompt = assemble(
                &input(&[], &items, &masking, &[], "none"),
                &ModelBudget::llama_cpp_8k(),
            );
            assert!(
                pairs_complete(&items, &prompt.tail_seqs),
                "turn {turn}: {:?}",
                prompt.tail_seqs
            );
        }
    }

    /// Review finding 8: whenever assembly has to leave a summary out, the planner merges.
    #[test]
    fn small_profiles_merge_before_assembly_drops_a_summary() {
        let config = CompactionConfig::small_model();
        for budget in [ModelBudget::mlx_4k(), ModelBudget::llama_cpp_8k()] {
            let window = budget.history_window();
            let now = NowBlock {
                as_of: 300,
                goal: "Plan the trip".into(),
                next_steps: Vec::new(),
                text: "x".repeat(config.now_budget(window) as usize * 4),
            };
            for claims in 1..40 {
                let summaries = [summary(1, 50, claims), summary(51, 100, claims)];
                let masking = Masking::default();
                let mut prompt_input = input(&summaries, &[], &masking, &[], "none");
                prompt_input.now_block = Some(&now);
                let prompt = assemble(&prompt_input, &budget);
                if !prompt.report.dropped_summaries.is_empty() {
                    assert!(
                        crate::summary::plan_merge(&summaries, window, &config).is_some(),
                        "{:?}: {claims} claims each are dropped but never merged",
                        budget.profile
                    );
                }
            }
        }
    }

    /// Review finding 8: a small hosted window still leaves room for history.
    #[test]
    fn small_hosted_windows_leave_history_room() {
        for (window, output) in [(4_096, 2_048), (8_192, 1_000), (16_384, 4_096)] {
            let budget = ModelBudget::hosted(window, output);
            assert!(
                budget.history_window() >= budget.usable_window() / 2,
                "{window}/{output}: H = {} of {}",
                budget.history_window(),
                budget.usable_window()
            );
        }
        let large = ModelBudget::hosted(200_000, 16_000);
        assert_eq!((large.about_state, large.facts_tokens), (4_100, 1_000));
    }

    #[test]
    fn empty_sections_add_no_headings() {
        let masking = Masking::default();
        let mut quiet = input(&[], &[], &masking, &[], "  ");
        quiet.about_you = "";
        let prompt = assemble(&quiet, &ModelBudget::hosted(200_000, 16_000));
        for kind in [
            SectionKind::AboutYou,
            SectionKind::Summaries,
            SectionKind::State,
            SectionKind::Memory,
        ] {
            assert_eq!(prompt.section(kind).unwrap().text, "", "{kind:?}");
        }
    }
}

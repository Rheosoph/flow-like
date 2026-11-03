use serde::{Deserialize, Serialize};

/// Compaction thresholds and budgets (research §3.1, §3.2). Shares are fractions of the history
/// window H (see [`history_window`]). The numbers follow the sources but are untuned defaults.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CompactionConfig {
    /// Tool results that stay in full; older ones become placeholders.
    pub keep_tool_results: usize,
    /// A result of an earlier turn above this share becomes a placeholder even among the
    /// last `keep_tool_results`; the current turn always sees its own results.
    pub large_result_share: f64,
    /// Compact in the background after a turn above this share.
    pub soft_trigger: f64,
    /// Compact while idle above this share, before the provider cache expires.
    pub idle_trigger: f64,
    /// Compact before the next call above this share.
    pub hard_trigger: f64,
    /// What a compaction aims to get below.
    pub target: f64,
    pub tail_turns_min: usize,
    pub tail_turns_max: usize,
    /// The verbatim tail's share at compaction time.
    pub tail_share: f64,
    /// Active summaries above this share get merged one level up.
    pub summary_merge_share: f64,
    pub now_block_tokens: u32,
    /// One level-0 summary's share, clamped to the two limits below.
    pub segment_summary_share: f64,
    pub segment_summary_min_tokens: u32,
    pub segment_summary_max_tokens: u32,
    /// A gap longer than this starts a new segment.
    pub segment_idle_gap_hours: i64,
    /// Characters of a tool result's gist kept in its placeholder.
    pub gist_chars: usize,
    /// The largest piece of history one summary or extraction call reads.
    pub chunk_tokens: u32,
    /// Messages before an extraction span the extractor sees as context.
    pub preceding_messages: usize,
}

impl Default for CompactionConfig {
    fn default() -> Self {
        Self {
            keep_tool_results: 3,
            large_result_share: 0.25,
            soft_trigger: 0.65,
            idle_trigger: 0.40,
            hard_trigger: 0.85,
            target: 0.40,
            tail_turns_min: 4,
            tail_turns_max: 6,
            tail_share: 0.25,
            summary_merge_share: 0.15,
            now_block_tokens: 300,
            segment_summary_share: 0.05,
            segment_summary_min_tokens: 120,
            segment_summary_max_tokens: 2_000,
            segment_idle_gap_hours: 6,
            gist_chars: 80,
            chunk_tokens: 16_000,
            preceding_messages: 10,
        }
    }
}

fn share(window: u32, fraction: f64) -> u32 {
    (f64::from(window) * fraction).round() as u32
}

impl CompactionConfig {
    /// Small local models read history in chunks of at most 2k tokens (research §3.7). Their
    /// prompts leave summaries 11–13 % of H once the Now block is in (8k: 500 of 3,973 tokens,
    /// 4k: 207 of 1,937), so summaries merge from 10 % of H instead of being left out.
    pub fn small_model() -> Self {
        Self {
            chunk_tokens: 2_000,
            summary_merge_share: 0.10,
            ..Self::default()
        }
    }

    pub fn large_result_limit(&self, window: u32) -> u32 {
        share(window, self.large_result_share)
    }

    pub fn soft_limit(&self, window: u32) -> u32 {
        share(window, self.soft_trigger)
    }

    pub fn idle_limit(&self, window: u32) -> u32 {
        share(window, self.idle_trigger)
    }

    pub fn hard_limit(&self, window: u32) -> u32 {
        share(window, self.hard_trigger)
    }

    pub fn target_tokens(&self, window: u32) -> u32 {
        share(window, self.target)
    }

    pub fn tail_budget(&self, window: u32) -> u32 {
        share(window, self.tail_share)
    }

    pub fn merge_threshold(&self, window: u32) -> u32 {
        share(window, self.summary_merge_share)
    }

    /// At most 300 tokens, and never more than a tenth of a small window.
    pub fn now_budget(&self, window: u32) -> u32 {
        self.now_block_tokens.min(window / 10)
    }

    pub fn segment_summary_budget(&self, window: u32) -> u32 {
        share(window, self.segment_summary_share).clamp(
            self.segment_summary_min_tokens,
            self.segment_summary_max_tokens
                .max(self.segment_summary_min_tokens),
        )
    }

    pub fn segment_idle_gap(&self) -> chrono::Duration {
        chrono::Duration::hours(self.segment_idle_gap_hours)
    }
}

/// W: the context window, capped at what the model handles well (local models: `n_ctx` and the
/// length it was shown to manage), minus the maximum output, minus 10 % of the window.
pub fn usable_window(context_window: u32, max_output: u32, effective_limit: Option<u32>) -> u32 {
    let context = effective_limit.map_or(context_window, |limit| context_window.min(limit));
    context
        .saturating_sub(max_output)
        .saturating_sub(context / 10)
}

/// H: what W leaves for history (summaries, the Now block and verbatim items) after the
/// sections compaction cannot shrink: system prompt and tools, about-you card, state block and
/// the reserve for recalled facts. For hosted models H is close to W; small models spend half
/// of W on those sections, so thresholds measured against W would compact every turn.
pub fn history_window(usable: u32, fixed_sections: u32) -> u32 {
    usable.saturating_sub(fixed_sections)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_follow_the_research() {
        let config = CompactionConfig::default();
        assert_eq!(config.keep_tool_results, 3);
        assert_eq!(config.soft_limit(1_000), 650);
        assert_eq!(config.idle_limit(1_000), 400);
        assert_eq!(config.hard_limit(1_000), 850);
        assert_eq!(config.target_tokens(1_000), 400);
        assert_eq!(config.tail_budget(1_000), 250);
        assert_eq!(config.merge_threshold(1_000), 150);
        assert_eq!((config.tail_turns_min, config.tail_turns_max), (4, 6));
        assert_eq!(config.segment_idle_gap(), chrono::Duration::hours(6));
        assert_eq!(CompactionConfig::small_model().chunk_tokens, 2_000);
    }

    #[test]
    fn windows_reserve_output_and_ten_percent() {
        assert_eq!(usable_window(200_000, 16_000, None), 164_000);
        assert_eq!(usable_window(8_192, 1_000, None), 6_373);
        assert_eq!(usable_window(131_072, 1_000, Some(32_768)), 28_492);
        assert_eq!(usable_window(1_000, 2_000, None), 0);
        assert_eq!(history_window(6_373, 2_400), 3_973);
        let config = CompactionConfig::default();
        assert_eq!(config.now_budget(164_000), 300);
        assert_eq!(config.now_budget(1_936), 193);
        assert_eq!(config.segment_summary_budget(164_000), 2_000);
        assert_eq!(config.segment_summary_budget(1_000), 120);
        assert_eq!(config.segment_summary_budget(10_000), 500);
    }
}

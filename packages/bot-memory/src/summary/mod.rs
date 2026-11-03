//! Segment summaries: level 0 for one evicted span, written once; higher levels merge the oldest
//! summaries and keep their children's ranges. Every claim cites the `#seq` it rests on.
//!
//! ```text
//! [#4800–#5120 · 2026-10-02..10-04] Planning the Atlas launch
//! Decisions:
//! - Launch moves to 2026-11-03 (#4810, #4815)
//! - UNVERIFIED: Finance approved the budget (#4816)
//! Superseded:
//! - Launch on 2026-11-01 (#4802)
//! ```

mod now;
mod prompt;

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::model::{Seq, SeqSpan};
use crate::pointer::{Pointer, scan_pointers};
use crate::text::{estimate_tokens, format_day_range, normalize, one_line, truncate_chars};

pub use now::{NowBlock, NowInput, now_request, parse_now_answer, write_now_block};
pub(crate) use prompt::chunk_lines;
pub use prompt::{
    SUMMARY_PROMPT_VERSION, SpanSummaryInput, merge_request, merge_summaries, plan_merge,
    render_span_item, segment_summary_request, summarize_span, summary_schema,
};

/// The fixed sections, in render order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Section {
    Decisions,
    Preferences,
    Tasks,
    Artifacts,
    OpenQuestions,
    Superseded,
}

impl Section {
    pub const ALL: [Section; 6] = [
        Section::Decisions,
        Section::Preferences,
        Section::Tasks,
        Section::Artifacts,
        Section::OpenQuestions,
        Section::Superseded,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Self::Decisions => "Decisions",
            Self::Preferences => "Preferences/instructions",
            Self::Tasks => "Tasks opened/closed",
            Self::Artifacts => "Artifacts with handles",
            Self::OpenQuestions => "Open questions",
            Self::Superseded => "Superseded",
        }
    }

    /// The JSON key in model answers.
    pub fn key(self) -> &'static str {
        match self {
            Self::Decisions => "decisions",
            Self::Preferences => "preferences",
            Self::Tasks => "tasks",
            Self::Artifacts => "artifacts",
            Self::OpenQuestions => "open_questions",
            Self::Superseded => "superseded",
        }
    }

    fn from_title(title: &str) -> Option<Self> {
        let title = normalize(title);
        Self::ALL.into_iter().find(|section| {
            normalize(section.title()) == title || normalize(section.key()) == title
        })
    }

    /// Over budget, claims go in this order: superseded first, artifact handles last.
    fn trim_rank(self) -> u8 {
        match self {
            Self::Superseded => 0,
            Self::OpenQuestions => 1,
            Self::Tasks => 2,
            Self::Decisions => 3,
            Self::Preferences => 4,
            Self::Artifacts => 5,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Claim {
    pub text: String,
    #[serde(default)]
    pub seqs: Vec<Seq>,
    #[serde(default)]
    pub unverified: bool,
}

impl Claim {
    pub fn newest(&self) -> Option<Seq> {
        self.seqs.iter().max().copied()
    }

    /// `- UNVERIFIED: text (#4810, #4812)`
    pub fn render(&self) -> String {
        let mut line = String::from("- ");
        if self.unverified {
            line.push_str("UNVERIFIED: ");
        }
        line.push_str(&self.text);
        if !self.seqs.is_empty() {
            let cites: Vec<String> = self.seqs.iter().map(|seq| format!("#{seq}")).collect();
            line.push_str(&format!(" ({})", cites.join(", ")));
        }
        line
    }
}

/// A summary's title and sections.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SummaryBody {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub sections: BTreeMap<Section, Vec<Claim>>,
}

impl SummaryBody {
    pub fn claims(&self) -> impl Iterator<Item = (Section, &Claim)> {
        self.sections
            .iter()
            .flat_map(|(section, claims)| claims.iter().map(move |claim| (*section, claim)))
    }

    pub fn claim_count(&self) -> usize {
        self.sections.values().map(Vec::len).sum()
    }

    pub fn push(&mut self, section: Section, claim: Claim) {
        self.sections.entry(section).or_default().push(claim);
    }

    pub fn citations(&self) -> BTreeSet<Seq> {
        self.claims()
            .flat_map(|(_, claim)| claim.seqs.iter().copied())
            .collect()
    }

    pub fn render_sections(&self) -> String {
        let mut text = String::new();
        for section in Section::ALL {
            let Some(claims) = self
                .sections
                .get(&section)
                .filter(|claims| !claims.is_empty())
            else {
                continue;
            };
            text.push_str(section.title());
            text.push_str(":\n");
            for claim in claims {
                text.push_str(&claim.render());
                text.push('\n');
            }
        }
        text.truncate(text.trim_end().len());
        text
    }

    /// Adds another body's claims; the first non-empty title stays.
    pub fn absorb(&mut self, other: SummaryBody) {
        if self.title.is_empty() {
            self.title = other.title;
        }
        for (section, claims) in other.sections {
            self.sections.entry(section).or_default().extend(claims);
        }
    }
}

/// Reads sections and claim lines from the text format, header optional: the fallback for
/// models that answer in prose instead of JSON.
pub fn parse_summary_sections(text: &str) -> SummaryBody {
    let mut body = SummaryBody::default();
    let mut section = None;
    for line in text.lines().map(str::trim).filter(|line| !line.is_empty()) {
        if line.starts_with('[') && line.contains(']') {
            body.title = line[line.find(']').map_or(0, |end| end + 1)..]
                .trim()
                .to_string();
            continue;
        }
        let bullet = ["- ", "* ", "• "]
            .into_iter()
            .find_map(|bullet| line.strip_prefix(bullet));
        match (bullet, section) {
            (Some(content), Some(current)) => body.push(current, parse_claim_line(content)),
            (Some(_), None) => {}
            (None, _) => {
                if let Some(title) = line.strip_suffix(':') {
                    section = Section::from_title(title);
                }
            }
        }
    }
    body
}

fn parse_claim_line(content: &str) -> Claim {
    const UNVERIFIED: &str = "UNVERIFIED:";
    let mut text = content.trim();
    let mut unverified = false;
    if let Some(prefix) = text.get(..UNVERIFIED.len())
        && prefix.eq_ignore_ascii_case(UNVERIFIED)
    {
        unverified = true;
        text = text[UNVERIFIED.len()..].trim_start();
    }
    let mut seqs = Vec::new();
    if text.ends_with(')')
        && let Some(open) = text.rfind('(')
    {
        let cites = scan_pointers(&text[open..]);
        if !cites.is_empty() {
            for (_, pointer) in cites {
                match pointer {
                    Pointer::Range { from, to } => seqs.extend([from, to]),
                    other => seqs.push(other.first_seq()),
                }
            }
            text = text[..open].trim_end();
        }
    }
    Claim {
        text: text.to_string(),
        seqs,
        unverified,
    }
}

/// Seq numbers from model JSON: numbers, `"#4812"` or `"4812"`.
pub(crate) fn seq_values(values: &[Value]) -> Vec<Seq> {
    values
        .iter()
        .filter_map(|value| match value {
            Value::Number(number) => number.as_u64(),
            Value::String(text) => Pointer::parse(text)
                .ok()
                .map(Pointer::first_seq)
                .or_else(|| text.trim().parse().ok()),
            _ => None,
        })
        .collect()
}

const MAX_CLAIM_CHARS: usize = 400;

/// Keeps citations to `allowed` positions only; a claim left without one is marked UNVERIFIED.
/// A preference or instruction needs a cited message of the user's own (`user`): tool output,
/// web pages and the bot's guesses never become one.
pub(crate) fn validate_body(
    body: SummaryBody,
    allowed: &BTreeSet<Seq>,
    user: &BTreeSet<Seq>,
) -> SummaryBody {
    let mut valid = SummaryBody {
        title: truncate_chars(&one_line(&body.title), 80),
        sections: BTreeMap::new(),
    };
    for (section, claims) in body.sections {
        for claim in claims {
            let text = truncate_chars(&one_line(&claim.text), MAX_CLAIM_CHARS);
            if text.is_empty() {
                continue;
            }
            let mut seqs: Vec<Seq> = claim
                .seqs
                .into_iter()
                .filter(|seq| allowed.contains(seq))
                .collect();
            seqs.sort_unstable();
            seqs.dedup();
            if section == Section::Preferences && !seqs.iter().any(|seq| user.contains(seq)) {
                continue;
            }
            let unverified = claim.unverified || seqs.is_empty();
            valid.push(
                section,
                Claim {
                    text,
                    seqs,
                    unverified,
                },
            );
        }
    }
    valid
}

/// The newest statement wins: a claim also listed as superseded leaves its section, equal claims
/// merge their citations, and every section runs oldest to newest.
pub(crate) fn consolidate(body: SummaryBody) -> SummaryBody {
    let superseded: BTreeSet<String> = body
        .sections
        .get(&Section::Superseded)
        .map(|claims| claims.iter().map(|claim| normalize(&claim.text)).collect())
        .unwrap_or_default();
    let mut sections = BTreeMap::new();
    for (section, claims) in body.sections {
        let mut merged: Vec<Claim> = Vec::new();
        for claim in claims {
            let key = normalize(&claim.text);
            if section != Section::Superseded && superseded.contains(&key) {
                continue;
            }
            match merged
                .iter_mut()
                .find(|existing| normalize(&existing.text) == key)
            {
                Some(existing) => {
                    existing.seqs.extend(claim.seqs);
                    existing.seqs.sort_unstable();
                    existing.seqs.dedup();
                    existing.unverified &= claim.unverified;
                }
                None => merged.push(claim),
            }
        }
        merged.sort_by_key(|claim| claim.newest().unwrap_or(Seq::MAX));
        if !merged.is_empty() {
            sections.insert(section, merged);
        }
    }
    SummaryBody {
        title: body.title,
        sections,
    }
}

/// The summary of one segment or, from level 1, of consecutive summaries.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SegmentSummary {
    pub id: String,
    pub level: u8,
    pub span: SeqSpan,
    pub t_from: DateTime<Utc>,
    pub t_to: DateTime<Utc>,
    pub body: SummaryBody,
    #[serde(default)]
    pub children: Vec<String>,
    #[serde(default)]
    pub child_spans: Vec<SeqSpan>,
    /// The cited positions that are the user's own messages: what a preference may rest on in
    /// a merge.
    #[serde(default)]
    pub user_seqs: Vec<Seq>,
    /// Ranges the model gave no usable answer for yet; `MemoryEngine::rewrite_pending` fills
    /// them in. The rendered summary names them, so the bot can read them itself.
    #[serde(default)]
    pub missing: Vec<SeqSpan>,
    /// The higher-level summary this one was merged into; `None` while it is active.
    #[serde(default)]
    pub parent: Option<String>,
    /// Bumped when the summary is rewritten from raw history (deletions, prompt or model change).
    pub version: u32,
    pub prompt_version: u32,
    #[serde(default)]
    pub model: Option<String>,
    pub created_at: DateTime<Utc>,
}

impl SegmentSummary {
    /// `s0-4800-5120`: stable, so a retried compaction overwrites instead of duplicating.
    pub fn id_for(level: u8, span: SeqSpan) -> String {
        format!("s{level}-{}-{}", span.from, span.to)
    }

    /// `[#4800–#5120 · 2026-10-02..10-04]`, from level 1 with `· L1 of 3 · expand s1-…`.
    pub fn header(&self, tz: Tz) -> String {
        let mut header = format!(
            "[{} · {}",
            self.span,
            format_day_range(self.t_from, self.t_to, tz)
        );
        if self.level > 0 {
            header.push_str(&format!(
                " · L{} of {} · expand {}",
                self.level,
                self.children.len(),
                self.id
            ));
        }
        header.push(']');
        if !self.body.title.is_empty() {
            header.push(' ');
            header.push_str(&self.body.title);
        }
        header
    }

    pub fn render(&self, tz: Tz) -> String {
        let mut text = self.header(tz);
        let sections = self.body.render_sections();
        if !sections.is_empty() {
            text.push('\n');
            text.push_str(&sections);
        }
        if !self.missing.is_empty() {
            let spans: Vec<String> = self.missing.iter().map(ToString::to_string).collect();
            text.push_str(&format!(
                "\nNot summarised yet (read with history_read): {}",
                spans.join(", ")
            ));
        }
        text
    }

    pub fn tokens(&self) -> u32 {
        estimate_tokens(&self.render(Tz::UTC))
    }

    pub fn is_active(&self) -> bool {
        self.parent.is_none()
    }

    /// What stays when no model can write the summary again after a deletion: the claims
    /// citing only remaining positions, without the title and the uncited claims (either may
    /// repeat the deleted text). A new version; the rewrite comes later.
    pub(crate) fn scrubbed(&self, deleted: &BTreeSet<Seq>) -> SegmentSummary {
        let mut body = SummaryBody::default();
        for (section, claim) in self.body.claims() {
            if !claim.seqs.is_empty() && claim.seqs.iter().all(|seq| !deleted.contains(seq)) {
                body.push(section, claim.clone());
            }
        }
        SegmentSummary {
            body,
            user_seqs: self
                .user_seqs
                .iter()
                .filter(|seq| !deleted.contains(seq))
                .copied()
                .collect(),
            version: self.version + 1,
            ..self.clone()
        }
    }

    /// Sets `user_seqs` to the cited positions among `user`.
    pub(crate) fn note_user_citations(&mut self, user: &BTreeSet<Seq>) {
        self.user_seqs = self.body.citations().intersection(user).copied().collect();
    }

    /// Drops claims, superseded ones first and the oldest of each section first, until the
    /// summary fits `budget` tokens.
    pub(crate) fn trim_to(&mut self, budget: u32) {
        while self.tokens() > budget {
            let Some(section) = Section::ALL
                .into_iter()
                .filter(|section| {
                    self.body
                        .sections
                        .get(section)
                        .is_some_and(|claims| !claims.is_empty())
                })
                .min_by_key(|section| section.trim_rank())
            else {
                break;
            };
            if let Some(claims) = self.body.sections.get_mut(&section) {
                claims.remove(0);
                if claims.is_empty() {
                    self.body.sections.remove(&section);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn claim(text: &str, seqs: &[Seq]) -> Claim {
        Claim {
            text: text.into(),
            seqs: seqs.to_vec(),
            unverified: false,
        }
    }

    fn sample(level: u8, from: Seq, to: Seq) -> SegmentSummary {
        let mut body = SummaryBody {
            title: "Planning the Atlas launch".into(),
            ..SummaryBody::default()
        };
        body.push(
            Section::Decisions,
            claim("Launch moves to 2026-11-03", &[from + 1, from + 2]),
        );
        body.push(
            Section::Decisions,
            Claim {
                unverified: true,
                ..claim("Finance approved the budget", &[from + 3])
            },
        );
        body.push(
            Section::Artifacts,
            claim("Plan in workspace://atlas/plan.md", &[from + 4]),
        );
        body.push(Section::Superseded, claim("Launch on 2026-11-01", &[from]));
        let span = SeqSpan::new(from, to);
        SegmentSummary {
            id: SegmentSummary::id_for(level, span),
            level,
            span,
            t_from: DateTime::parse_from_rfc3339("2026-10-02T09:00:00Z")
                .unwrap()
                .into(),
            t_to: DateTime::parse_from_rfc3339("2026-10-04T18:00:00Z")
                .unwrap()
                .into(),
            body,
            children: Vec::new(),
            child_spans: Vec::new(),
            user_seqs: Vec::new(),
            missing: Vec::new(),
            parent: None,
            version: 1,
            prompt_version: SUMMARY_PROMPT_VERSION,
            model: None,
            created_at: DateTime::parse_from_rfc3339("2026-10-05T00:00:00Z")
                .unwrap()
                .into(),
        }
    }

    #[test]
    fn summaries_render_the_research_format_and_parse_back() {
        let summary = sample(0, 4800, 5120);
        let text = summary.render(Tz::UTC);
        assert_eq!(
            text,
            "[#4800–#5120 · 2026-10-02..10-04] Planning the Atlas launch\n\
             Decisions:\n\
             - Launch moves to 2026-11-03 (#4801, #4802)\n\
             - UNVERIFIED: Finance approved the budget (#4803)\n\
             Artifacts with handles:\n\
             - Plan in workspace://atlas/plan.md (#4804)\n\
             Superseded:\n\
             - Launch on 2026-11-01 (#4800)"
        );
        assert_eq!(parse_summary_sections(&text), summary.body);
        assert_eq!(
            summary.body.citations().into_iter().collect::<Vec<_>>(),
            [4800, 4801, 4802, 4803, 4804]
        );
    }

    #[test]
    fn higher_levels_name_their_children_in_the_header() {
        let mut summary = sample(1, 100, 900);
        summary.children = vec!["s0-100-400".into(), "s0-401-900".into()];
        assert_eq!(
            summary.header(Tz::UTC),
            "[#100–#900 · 2026-10-02..10-04 · L1 of 2 · expand s1-100-900] Planning the Atlas launch"
        );
    }

    #[test]
    fn citations_outside_the_span_are_dropped_and_leave_claims_unverified() {
        let mut body = SummaryBody::default();
        body.push(Section::Decisions, claim("Kept", &[5, 99]));
        body.push(Section::Decisions, claim("Invented", &[99]));
        body.push(Section::Decisions, claim("   ", &[5]));
        let allowed: BTreeSet<Seq> = [5, 6].into_iter().collect();
        let valid = validate_body(body, &allowed, &BTreeSet::new());
        let claims = &valid.sections[&Section::Decisions];
        assert_eq!(claims.len(), 2);
        assert_eq!(claims[0], claim("Kept", &[5]));
        assert!(claims[1].unverified && claims[1].seqs.is_empty());
    }

    #[test]
    fn the_newest_statement_wins_and_equal_claims_merge() {
        let mut body = SummaryBody::default();
        body.push(Section::Decisions, claim("Launch on 2026-11-01", &[2]));
        body.push(Section::Decisions, claim("Use Postgres", &[9]));
        body.push(Section::Decisions, claim("use postgres.", &[4]));
        body.push(Section::Superseded, claim("Launch on 2026-11-01", &[2]));
        body.push(Section::Decisions, claim("Launch on 2026-11-03", &[7]));
        let consolidated = consolidate(body);
        assert_eq!(
            consolidated.sections[&Section::Decisions],
            [
                claim("Launch on 2026-11-03", &[7]),
                claim("Use Postgres", &[4, 9])
            ]
        );
        assert_eq!(consolidated.sections[&Section::Superseded].len(), 1);
    }

    #[test]
    fn trimming_drops_superseded_and_oldest_claims_first() {
        let mut summary = sample(0, 10, 20);
        let full = summary.tokens();
        summary.trim_to(full - 1);
        assert!(!summary.body.sections.contains_key(&Section::Superseded));
        assert_eq!(summary.body.claim_count(), 3);
        summary.trim_to(0);
        assert_eq!(summary.body.claim_count(), 0);
        assert!(summary.render(Tz::UTC).starts_with("[#10–#20"));
    }

    #[test]
    fn prose_answers_parse_with_bullets_and_ranges() {
        let body = parse_summary_sections(
            "Here you go.\nDecisions:\n* Ship it (#4–#6)\n• UNVERIFIED: maybe later\nNot a section:\n- ignored\nOpen questions:\n- Which region? (#7)",
        );
        assert_eq!(
            body.sections[&Section::Decisions][0],
            claim("Ship it", &[4, 6])
        );
        assert!(body.sections[&Section::Decisions][1].unverified);
        assert_eq!(
            body.sections[&Section::OpenQuestions],
            [claim("Which region?", &[7])]
        );
        assert_eq!(body.claim_count(), 3);
    }

    #[test]
    fn seq_values_accept_numbers_and_pointers() {
        let values = serde_json::json!([4812, "#4813", "4814", "#4815.t1", "nope", -1]);
        assert_eq!(
            seq_values(values.as_array().unwrap()),
            [4812, 4813, 4814, 4815]
        );
    }
}

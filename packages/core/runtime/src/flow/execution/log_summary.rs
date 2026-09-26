//! Per-run log summary: level and node counts, the first error, and repeat
//! groups keyed by a message fingerprint. It is accumulated while a run
//! flushes its logs and stored as a sidecar next to the run's log table
//! (`runs/{app_id}/{board_id}/{run_id}.summary.json`), so opening the log
//! panel never scans the table for counts.
//!
//! A fingerprint masks what varies between repeats of one message — numbers
//! (`⟨n⟩`), hex ids and UUIDs (`⟨id⟩`) — and hashes the result together with
//! the node and level. Quoted text is kept: two errors wrapping different
//! causes are different problems.

use std::collections::{BTreeMap, HashMap};

use flow_like_storage::Path;
use flow_like_storage::files::store::FlowLikeStore;
use flow_like_storage::object_store::{self, ObjectStoreExt};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::run_index::runs_base_path;

pub const SUMMARY_VERSION: u32 = 1;
pub const NUMBER_SLOT: &str = "⟨n⟩";
pub const ID_SLOT: &str = "⟨id⟩";

const LEVELS: usize = 5;
const MAX_TRACKED_GROUPS: usize = 2_000;
const MAX_REPORTED_GROUPS: usize = 250;
const MAX_SLOTS: usize = 8;
const MAX_TEMPLATE_CHARS: usize = 512;
const MAX_SAMPLE_CHARS: usize = 4_096;
const UNIT_SUFFIX_MAX: usize = 4;

#[derive(Debug, Clone, PartialEq)]
pub struct Fingerprint {
    pub id: String,
    pub template: String,
    pub numbers: Vec<f64>,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, PartialEq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct LogSlotRange {
    pub min: f64,
    pub max: f64,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, PartialEq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SummaryLog {
    pub node_id: Option<String>,
    pub log_level: u8,
    pub start: u64,
    pub message: String,
    #[serde(default)]
    pub fingerprint: Option<String>,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, PartialEq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct LogGroup {
    pub fingerprint: String,
    pub node_id: Option<String>,
    pub log_level: u8,
    pub template: String,
    /// The i-th entry ranges over the i-th `⟨n⟩` of `template`; later slots are not tracked.
    pub slots: Vec<Option<LogSlotRange>>,
    pub count: u64,
    pub first_start: u64,
    pub last_start: u64,
    pub sample: String,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, PartialEq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct LogSummary {
    pub version: u32,
    /// Rows carry a `fingerprint` column, so groups can be folded and filtered.
    pub fingerprinted: bool,
    /// Computed from a capped scan; counts are lower bounds.
    pub partial: bool,
    pub total: u64,
    pub levels: [u64; LEVELS],
    pub nodes: BTreeMap<String, [u64; LEVELS]>,
    pub first_error: Option<SummaryLog>,
    pub visited: Option<Vec<String>>,
    pub groups: Vec<LogGroup>,
    pub groups_truncated: bool,
}

#[derive(Clone)]
struct GroupAcc {
    node_id: Option<String>,
    log_level: u8,
    template: String,
    slots: Vec<Option<LogSlotRange>>,
    count: u64,
    first_start: u64,
    last_start: u64,
    sample: String,
}

#[derive(Default, Clone)]
pub struct LogSummaryBuilder {
    total: u64,
    levels: [u64; LEVELS],
    nodes: HashMap<String, [u64; LEVELS]>,
    first_error: Option<SummaryLog>,
    groups: HashMap<String, GroupAcc>,
    groups_truncated: bool,
}

impl LogSummaryBuilder {
    pub fn record(
        &mut self,
        node_id: Option<&str>,
        log_level: u8,
        start: u64,
        message: &str,
        fingerprint: Option<&Fingerprint>,
    ) {
        let level = usize::from(log_level).min(LEVELS - 1);
        self.total += 1;
        self.levels[level] += 1;
        self.nodes
            .entry(node_id.unwrap_or_default().to_string())
            .or_default()[level] += 1;

        if log_level >= 3
            && self
                .first_error
                .as_ref()
                .is_none_or(|first| start < first.start)
        {
            self.first_error = Some(SummaryLog {
                node_id: node_id.map(str::to_string),
                log_level,
                start,
                message: truncate_chars(message, MAX_SAMPLE_CHARS),
                fingerprint: fingerprint.map(|fp| fp.id.clone()),
            });
        }

        let Some(fingerprint) = fingerprint else {
            return;
        };
        if let Some(group) = self.groups.get_mut(&fingerprint.id) {
            group.count += 1;
            if start < group.first_start {
                group.first_start = start;
                group.sample = truncate_chars(message, MAX_SAMPLE_CHARS);
            }
            group.last_start = group.last_start.max(start);
            widen_slots(&mut group.slots, &fingerprint.numbers);
            return;
        }
        if self.groups.len() >= MAX_TRACKED_GROUPS {
            self.groups_truncated = true;
            return;
        }
        let mut slots = Vec::new();
        widen_slots(&mut slots, &fingerprint.numbers);
        self.groups.insert(
            fingerprint.id.clone(),
            GroupAcc {
                node_id: node_id.map(str::to_string),
                log_level,
                template: truncate_chars(&fingerprint.template, MAX_TEMPLATE_CHARS),
                slots,
                count: 1,
                first_start: start,
                last_start: start,
                sample: truncate_chars(message, MAX_SAMPLE_CHARS),
            },
        );
    }

    pub fn is_empty(&self) -> bool {
        self.total == 0
    }

    pub fn finish(&self, fingerprinted: bool, visited: Option<Vec<String>>) -> LogSummary {
        let mut groups = self
            .groups
            .iter()
            .map(|(fingerprint, acc)| LogGroup {
                fingerprint: fingerprint.clone(),
                node_id: acc.node_id.clone(),
                log_level: acc.log_level,
                template: acc.template.clone(),
                slots: acc.slots.clone(),
                count: acc.count,
                first_start: acc.first_start,
                last_start: acc.last_start,
                sample: acc.sample.clone(),
            })
            .collect::<Vec<_>>();
        groups.sort_by(|a, b| {
            b.count
                .cmp(&a.count)
                .then(a.first_start.cmp(&b.first_start))
        });
        let groups_truncated = self.groups_truncated || groups.len() > MAX_REPORTED_GROUPS;
        groups.truncate(MAX_REPORTED_GROUPS);

        let mut visited = visited;
        if let Some(visited) = visited.as_mut() {
            visited.sort();
        }

        LogSummary {
            version: SUMMARY_VERSION,
            fingerprinted,
            partial: false,
            total: self.total,
            levels: self.levels,
            nodes: self
                .nodes
                .iter()
                .map(|(node, counts)| (node.clone(), *counts))
                .collect(),
            first_error: self.first_error.clone(),
            visited,
            groups,
            groups_truncated,
        }
    }
}

fn widen_slots(slots: &mut Vec<Option<LogSlotRange>>, numbers: &[f64]) {
    for (index, value) in numbers.iter().take(MAX_SLOTS).enumerate() {
        if !value.is_finite() {
            continue;
        }
        if index >= slots.len() {
            slots.resize(index + 1, None);
        }
        match slots[index].as_mut() {
            Some(range) => {
                range.min = range.min.min(*value);
                range.max = range.max.max(*value);
            }
            None => {
                slots[index] = Some(LogSlotRange {
                    min: *value,
                    max: *value,
                })
            }
        }
    }
}

fn truncate_chars(text: &str, max: usize) -> String {
    match text.char_indices().nth(max) {
        Some((cut, _)) => format!("{}…", &text[..cut]),
        None => text.to_string(),
    }
}

pub fn fingerprint(node_id: Option<&str>, log_level: u8, message: &str) -> Fingerprint {
    let (template, numbers) = mask_message(message);
    let mut hash = Fnv64::new();
    hash.write(node_id.unwrap_or_default().as_bytes());
    hash.write(&[0x1f, log_level, 0x1f]);
    hash.write(template.as_bytes());
    Fingerprint {
        id: format!("{:016x}", hash.finish()),
        template,
        numbers,
    }
}

struct Fnv64(u64);

impl Fnv64 {
    fn new() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }

    fn write(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.0 ^= u64::from(*byte);
            self.0 = self.0.wrapping_mul(0x0100_0000_01b3);
        }
    }

    fn finish(&self) -> u64 {
        self.0
    }
}

fn is_ident(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn is_ascii_ident(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

fn looks_like_id(token: &str) -> bool {
    let digits = token.bytes().filter(u8::is_ascii_digit).count();
    if digits == 0 {
        return false;
    }
    if token.len() >= 8 && token.bytes().all(|b| b.is_ascii_hexdigit()) {
        return true;
    }
    token.len() >= 12 && digits >= 3 && token.bytes().any(|b| b.is_ascii_alphabetic())
}

fn uuid_at(chars: &[char], at: usize) -> bool {
    const GROUPS: [usize; 5] = [8, 4, 4, 4, 12];
    let mut index = at;
    for (group_index, len) in GROUPS.iter().enumerate() {
        for _ in 0..*len {
            if !chars.get(index).is_some_and(char::is_ascii_hexdigit) {
                return false;
            }
            index += 1;
        }
        if group_index < GROUPS.len() - 1 {
            if chars.get(index) != Some(&'-') {
                return false;
            }
            index += 1;
        }
    }
    !chars.get(index).copied().is_some_and(is_ident)
}

/// Replaces numbers with `⟨n⟩` and ids with `⟨id⟩`, returning the template
/// and the masked numbers in order. Digits inside words (`utf8`) are kept.
pub fn mask_message(message: &str) -> (String, Vec<f64>) {
    let chars = message.chars().collect::<Vec<_>>();
    let mut template = String::with_capacity(message.len());
    let mut numbers = Vec::new();
    let mut prev_ident = false;
    let mut i = 0;

    while i < chars.len() {
        let c = chars[i];
        if prev_ident || !is_ascii_ident(c) {
            template.push(c);
            prev_ident = is_ident(c);
            i += 1;
            continue;
        }

        if c.is_ascii_hexdigit() && uuid_at(&chars, i) {
            template.push_str(ID_SLOT);
            i += 36;
            prev_ident = false;
            continue;
        }

        let token_end = scan_while(&chars, i, is_ascii_ident);
        let token = chars[i..token_end].iter().collect::<String>();

        if c.is_ascii_digit() {
            if token.len() > 2
                && (token.starts_with("0x") || token.starts_with("0X"))
                && token[2..].bytes().all(|b| b.is_ascii_hexdigit())
            {
                template.push_str(ID_SLOT);
                i = token_end;
                prev_ident = false;
                continue;
            }

            let int_end = scan_while(&chars, i, |c| c.is_ascii_digit());
            let mut number_end = int_end;
            if int_end == token_end
                && chars.get(int_end) == Some(&'.')
                && chars.get(int_end + 1).is_some_and(char::is_ascii_digit)
            {
                number_end = scan_while(&chars, int_end + 1, |c| c.is_ascii_digit());
            }
            let rest_end = scan_while(&chars, number_end, is_ascii_ident);
            let rest = chars[number_end..rest_end].iter().collect::<String>();

            if rest.is_empty()
                || (rest.len() <= UNIT_SUFFIX_MAX && rest.bytes().all(|b| b.is_ascii_alphabetic()))
            {
                let digits = chars[i..number_end].iter().collect::<String>();
                numbers.push(digits.parse::<f64>().unwrap_or(f64::NAN));
                template.push_str(NUMBER_SLOT);
                template.push_str(&rest);
                i = rest_end;
                prev_ident = !rest.is_empty();
                continue;
            }
        }

        if looks_like_id(&token) {
            template.push_str(ID_SLOT);
        } else {
            template.push_str(&token);
        }
        i = token_end;
        prev_ident = true;
    }

    (template, numbers)
}

fn scan_while(chars: &[char], from: usize, pred: impl Fn(char) -> bool) -> usize {
    let mut end = from;
    while end < chars.len() && pred(chars[end]) {
        end += 1;
    }
    end
}

/// `runs/{app_id}/{board_id}/{run_id}.summary.json`, next to the run's log table.
pub fn run_summary_path(app_id: &str, board_id: &str, run_id: &str) -> Path {
    runs_base_path(app_id, board_id).join(format!("{run_id}.summary.json").as_str())
}

pub async fn write_run_summary(
    store: &FlowLikeStore,
    app_id: &str,
    board_id: &str,
    run_id: &str,
    summary: &LogSummary,
) -> flow_like_types::Result<()> {
    let bytes = flow_like_types::json::to_vec(summary)?;
    store
        .put(&run_summary_path(app_id, board_id, run_id), bytes)
        .await
}

/// `None` when the run predates summaries or recorded no logs.
pub async fn read_run_summary(
    store: &FlowLikeStore,
    app_id: &str,
    board_id: &str,
    run_id: &str,
) -> flow_like_types::Result<Option<LogSummary>> {
    match store
        .as_generic()
        .get(&run_summary_path(app_id, board_id, run_id))
        .await
    {
        Ok(result) => {
            let bytes = result.bytes().await?;
            let summary = flow_like_types::json::from_slice::<LogSummary>(&bytes).map_err(|e| {
                flow_like_types::anyhow!("Run {run_id} has an unreadable log summary: {e}")
            })?;
            Ok(Some(summary))
        }
        Err(object_store::Error::NotFound { .. }) => Ok(None),
        Err(error) => Err(error.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn template(message: &str) -> String {
        mask_message(message).0
    }

    #[test]
    fn masks_numbers_and_ids_but_keeps_words_and_quotes() {
        assert_eq!(
            template(
                "Error: ExecutionFailed(\"invalid type: null, expected struct NodeDBConnection\") in iteration 982"
            ),
            "Error: ExecutionFailed(\"invalid type: null, expected struct NodeDBConnection\") in iteration ⟨n⟩"
        );
        assert_eq!(
            template("Starting Node Execution: Set Field [3d8b7e05]"),
            "Starting Node Execution: Set Field [⟨id⟩]"
        );
        assert_eq!(
            template("run 550e8400-e29b-41d4-a716-446655440000 done"),
            "run ⟨id⟩ done"
        );
        assert_eq!(template("took 1.84s, 2.31 MB"), "took ⟨n⟩s, ⟨n⟩ MB");
        assert_eq!(template("utf8 and base64 stay"), "utf8 and base64 stay");
        assert_eq!(template("addr 0x7ffd1a2b"), "addr ⟨id⟩");
        assert_eq!(template("bbox 52.3383,13.0884"), "bbox ⟨n⟩,⟨n⟩");
        assert_eq!(template("op_7Hq2kd83ja9 failed"), "⟨id⟩ failed");
        assert_eq!(template("größe 12 äpfel"), "größe ⟨n⟩ äpfel");
    }

    #[test]
    fn records_masked_numbers_in_order() {
        let (_, numbers) = mask_message("iteration 7 of 10 took 1.5ms");
        assert_eq!(numbers, vec![7.0, 10.0, 1.5]);
    }

    #[test]
    fn fingerprint_separates_nodes_and_levels_and_groups_repeats() {
        let a = fingerprint(Some("n1"), 3, "failed in iteration 1");
        let b = fingerprint(Some("n1"), 3, "failed in iteration 2");
        let other_node = fingerprint(Some("n2"), 3, "failed in iteration 1");
        let other_level = fingerprint(Some("n1"), 2, "failed in iteration 1");
        assert_eq!(a.id, b.id);
        assert_ne!(a.id, other_node.id);
        assert_ne!(a.id, other_level.id);
        assert_eq!(a.id.len(), 16);
    }

    #[test]
    fn builder_counts_groups_first_error_and_slot_ranges() {
        let mut builder = LogSummaryBuilder::default();
        for i in 0..5u64 {
            let message = format!("iteration {i}");
            let fp = fingerprint(Some("loop"), 3, &message);
            builder.record(Some("loop"), 3, 100 + i, &message, Some(&fp));
        }
        let debug = fingerprint(Some("print"), 0, "hello");
        builder.record(Some("print"), 0, 50, "hello", Some(&debug));

        let summary = builder.finish(true, Some(vec!["print".into(), "loop".into()]));
        assert_eq!(summary.total, 6);
        assert_eq!(summary.levels, [1, 0, 0, 5, 0]);
        assert_eq!(summary.nodes["loop"], [0, 0, 0, 5, 0]);
        assert_eq!(summary.visited, Some(vec!["loop".into(), "print".into()]));
        let first = summary.first_error.unwrap();
        assert_eq!((first.start, first.message.as_str()), (100, "iteration 0"));
        let group = &summary.groups[0];
        assert_eq!(group.count, 5);
        assert_eq!(group.template, "iteration ⟨n⟩");
        assert_eq!((group.first_start, group.last_start), (100, 104));
        assert_eq!(group.slots, vec![Some(LogSlotRange { min: 0.0, max: 4.0 })]);
        assert_eq!(group.sample, "iteration 0");
    }

    #[test]
    fn builder_stops_tracking_new_groups_at_the_cap() {
        fn letters(mut n: usize) -> String {
            let mut word = String::new();
            loop {
                word.push((b'a' + (n % 26) as u8) as char);
                n /= 26;
                if n == 0 {
                    return word;
                }
            }
        }

        let mut builder = LogSummaryBuilder::default();
        for i in 0..(MAX_TRACKED_GROUPS + 5) {
            let message = format!("distinct message {}", letters(i));
            let fp = fingerprint(None, 1, &message);
            builder.record(None, 1, i as u64, &message, Some(&fp));
        }
        let summary = builder.finish(true, None);
        assert!(summary.groups_truncated);
        assert_eq!(summary.groups.len(), MAX_REPORTED_GROUPS);
        assert_eq!(summary.total, (MAX_TRACKED_GROUPS + 5) as u64);
    }

    #[test]
    fn summary_path_sits_next_to_the_run_table() {
        assert_eq!(
            run_summary_path("app", "board", "run").to_string(),
            "runs/app/board/run.summary.json"
        );
    }
}

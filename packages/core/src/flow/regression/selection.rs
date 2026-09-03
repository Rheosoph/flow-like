//! Corpus selection for regression suites: which recorded runs become
//! replay candidates. The corpus is the plaintext `LogMeta.payload` column of
//! the per-board Lance runs table — never the encrypted
//! `ExecutionRun.inputPayloadKey` blobs, which only a minority of run-creation
//! sites write.

use crate::flow::execution::{LogLevel, LogMeta};
use flow_like_types::Value;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashSet};
use std::future::Future;
use std::time::Duration;

/// Hard cap on rows fetched per scan. The Lance query builder has **no ORDER
/// BY**, so `limit(500)` returns an *arbitrary* 500 rows, not the newest —
/// selection must sort in memory and treat a capped scan as "widening the
/// window cannot help" (a wider window feeds the same arbitrary truncation).
pub const CORPUS_SCAN_CAP: usize = 500;

/// The widening ladder: 24h → 48h → 7d → 30d. 30 days is the agreed corpus
/// retention window, so the ladder never widens past it.
pub const CORPUS_WINDOWS: [Duration; 4] = [
    Duration::from_secs(24 * 3600),
    Duration::from_secs(48 * 3600),
    Duration::from_secs(7 * 24 * 3600),
    Duration::from_secs(30 * 24 * 3600),
];

/// How many recent suite runs contribute their replay run ids to the
/// exclusion set. Bounded work: 50 suite runs × ≤100 cases each. Without the
/// exclusion, "newest per shape" preferentially keeps a nightly suite's own
/// replays — the suite converges on testing itself.
pub const REPLAY_EXCLUSION_SUITE_RUNS: usize = 50;

/// One recorded run as selection consumes it, built from the Lance summary
/// row. `source_node_id` is the summary's `node_id` — for REST/MCP events
/// that is the registration's handler node, which is the node a replay must
/// dispatch into (a single event can fan out over N handler nodes).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CorpusCandidate {
    pub run_id: String,
    pub source_node_id: String,
    pub event_id: String,
    /// Unix micros, from `LogMeta.start`.
    pub start: u64,
    /// Unix micros, from `LogMeta.end`.
    pub end: u64,
    pub payload: Value,
    /// The recorded run's highest log level reached `Error` — stratification
    /// guarantees these rows are never selected away.
    pub failed: bool,
    pub visited_node_ids: Vec<String>,
}

impl CorpusCandidate {
    pub fn from_log_meta(meta: &LogMeta) -> Self {
        let payload = if meta.payload.is_empty() {
            Value::Null
        } else {
            flow_like_types::json::from_slice(&meta.payload).unwrap_or(Value::Null)
        };
        CorpusCandidate {
            run_id: meta.run_id.clone(),
            source_node_id: meta.node_id.clone(),
            event_id: meta.event_id.clone(),
            start: meta.start,
            end: meta.end,
            payload,
            failed: meta.log_level >= LogLevel::Error.to_u8(),
            visited_node_ids: meta
                .nodes
                .iter()
                .flatten()
                .map(|(node_id, _)| node_id.clone())
                .collect(),
        }
    }
}

/// The outcome of [`select_corpus_window`], carrying enough provenance for a
/// UI to explain the selection ("48h window, 137 rows scanned").
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CorpusSelection {
    pub candidates: Vec<CorpusCandidate>,
    /// The window the returned selection was drawn from.
    pub window: Duration,
    /// Raw rows the final scan returned, pre-refinement.
    pub scanned_rows: usize,
    /// The final scan hit [`CORPUS_SCAN_CAP`]. With no ORDER BY in the query
    /// builder the capped set is arbitrary, so widening stopped here.
    pub scan_capped: bool,
}

/// Structural hash of a payload: blake3 over the sorted, deduplicated set of
/// leaf key-paths plus their JSON types. Two payloads with the same keys and
/// types hash equal regardless of values, key order, or array length —
/// "newest per shape" dedup keys on this.
pub fn shape_hash(payload: &Value) -> String {
    let mut entries = BTreeSet::new();
    collect_shape(payload, "$", &mut entries);
    let joined = entries.into_iter().collect::<Vec<_>>().join("\n");
    blake3::hash(joined.as_bytes()).to_hex().to_string()
}

fn collect_shape(value: &Value, path: &str, out: &mut BTreeSet<String>) {
    match value {
        Value::Null => {
            out.insert(format!("{path}:null"));
        }
        Value::Bool(_) => {
            out.insert(format!("{path}:bool"));
        }
        Value::Number(_) => {
            out.insert(format!("{path}:number"));
        }
        Value::String(_) => {
            out.insert(format!("{path}:string"));
        }
        Value::Array(items) => {
            if items.is_empty() {
                out.insert(format!("{path}:array"));
            } else {
                let element_path = format!("{path}[]");
                for item in items {
                    collect_shape(item, &element_path, out);
                }
            }
        }
        Value::Object(map) => {
            if map.is_empty() {
                out.insert(format!("{path}:object"));
            } else {
                for (key, entry) in map {
                    collect_shape(entry, &format!("{path}.{key}"), out);
                }
            }
        }
    }
}

fn sort_newest_first(rows: &mut [CorpusCandidate]) {
    rows.sort_by_key(|row| std::cmp::Reverse((row.start, row.end)));
}

/// Drop rows whose run id is in the exclusion set — the replay run ids of the
/// last [`REPLAY_EXCLUSION_SUITE_RUNS`] suite runs. Applied **before** any
/// dedupe so a replay row can never displace the real recording it copied.
pub fn filter_excluded(
    mut rows: Vec<CorpusCandidate>,
    excluded_run_ids: &HashSet<String>,
) -> Vec<CorpusCandidate> {
    rows.retain(|row| !excluded_run_ids.contains(&row.run_id));
    rows
}

/// Keep the newest row per `run_id` — a double `LogMeta::flush` writes the
/// same run twice into the runs table.
pub fn dedupe_by_run_id(mut rows: Vec<CorpusCandidate>) -> Vec<CorpusCandidate> {
    sort_newest_first(&mut rows);
    let mut seen = HashSet::with_capacity(rows.len());
    rows.retain(|row| seen.insert(row.run_id.clone()));
    rows
}

/// Keep the newest row per payload shape (see [`shape_hash`]), so the
/// selection spreads over distinct input structures instead of drowning in
/// the highest-traffic one.
pub fn dedupe_by_shape(mut rows: Vec<CorpusCandidate>) -> Vec<CorpusCandidate> {
    sort_newest_first(&mut rows);
    let mut seen = HashSet::with_capacity(rows.len());
    rows.retain(|row| seen.insert(shape_hash(&row.payload)));
    rows
}

/// Trim to `target` without ever trimming away a failing input: failing rows
/// fill slots first (newest first), passing rows fill the rest, and the
/// result is returned newest-first. Failing recordings are first-class
/// fixtures — they are how `FIXED` becomes reportable.
pub fn stratify_failures(mut rows: Vec<CorpusCandidate>, target: usize) -> Vec<CorpusCandidate> {
    sort_newest_first(&mut rows);
    if rows.len() <= target {
        return rows;
    }
    let (failing, passing): (Vec<_>, Vec<_>) = rows.into_iter().partition(|row| row.failed);
    let mut selected: Vec<CorpusCandidate> = failing.into_iter().take(target).collect();
    let remaining = target.saturating_sub(selected.len());
    selected.extend(passing.into_iter().take(remaining));
    sort_newest_first(&mut selected);
    selected
}

/// The full refinement pipeline over one scan's rows: in-memory newest-first
/// ordering (the scan itself is unordered), exclusion filtering, run-id
/// dedupe, shape dedupe, failure-preserving trim to `target`.
pub fn refine_corpus_rows(
    rows: Vec<CorpusCandidate>,
    excluded_run_ids: &HashSet<String>,
    target: usize,
) -> Vec<CorpusCandidate> {
    let rows = filter_excluded(rows, excluded_run_ids);
    let rows = dedupe_by_run_id(rows);
    let rows = dedupe_by_shape(rows);
    stratify_failures(rows, target)
}

/// Select up to `target` corpus candidates, widening the scan window along
/// [`CORPUS_WINDOWS`] until the refined selection reaches `target` or a scan
/// hits [`CORPUS_SCAN_CAP`].
///
/// `scan(window, cap)` fetches at most `cap` rows recorded within `window` of
/// now — the caller owns the actual store query (Lance on both cloud and
/// desktop). **Caveat the scan cap exists for:** the Lance query builder has
/// no ORDER BY, so a capped result is an arbitrary subset; once a scan is
/// capped, widening further only re-truncates arbitrarily, so widening stops
/// and the capped window's refined selection is returned as-is.
pub async fn select_corpus_window<F, Fut>(
    target: usize,
    excluded_run_ids: &HashSet<String>,
    mut scan: F,
) -> flow_like_types::Result<CorpusSelection>
where
    F: FnMut(Duration, usize) -> Fut,
    Fut: Future<Output = flow_like_types::Result<Vec<CorpusCandidate>>>,
{
    let mut widest: Option<CorpusSelection> = None;
    for window in CORPUS_WINDOWS {
        let rows = scan(window, CORPUS_SCAN_CAP).await?;
        let scanned_rows = rows.len();
        let scan_capped = scanned_rows >= CORPUS_SCAN_CAP;
        let candidates = refine_corpus_rows(rows, excluded_run_ids, target);
        let selection = CorpusSelection {
            candidates,
            window,
            scanned_rows,
            scan_capped,
        };
        if selection.candidates.len() >= target || scan_capped {
            return Ok(selection);
        }
        widest = Some(selection);
    }
    widest.ok_or_else(|| flow_like_types::anyhow!("corpus window ladder is empty"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_types::json::json;
    use flow_like_types::tokio;
    use std::sync::{Arc, Mutex};

    fn candidate(run_id: &str, start: u64, payload: Value, failed: bool) -> CorpusCandidate {
        CorpusCandidate {
            run_id: run_id.to_string(),
            source_node_id: "node-1".to_string(),
            event_id: "event-1".to_string(),
            start,
            end: start + 10,
            payload,
            failed,
            visited_node_ids: vec![],
        }
    }

    #[test]
    fn shape_hash_ignores_values_key_order_and_array_length() {
        let a = json!({ "user": { "id": 1, "name": "a" }, "tags": ["x", "y"] });
        let b = json!({ "tags": ["z"], "user": { "name": "b", "id": 99 } });
        assert_eq!(shape_hash(&a), shape_hash(&b));
    }

    #[test]
    fn shape_hash_distinguishes_types_keys_and_containers() {
        let base = json!({ "id": 1 });
        assert_ne!(shape_hash(&base), shape_hash(&json!({ "id": "1" })));
        assert_ne!(shape_hash(&base), shape_hash(&json!({ "id2": 1 })));
        assert_ne!(shape_hash(&base), shape_hash(&json!({ "id": [1] })));
        assert_ne!(shape_hash(&json!({})), shape_hash(&json!([])));
        assert_ne!(shape_hash(&Value::Null), shape_hash(&json!({})));
    }

    #[test]
    fn from_log_meta_maps_summary_fields() {
        let meta = LogMeta {
            app_id: "app".into(),
            run_id: "run-1".into(),
            board_id: "board".into(),
            start: 100,
            end: 200,
            log_level: LogLevel::Error.to_u8(),
            version: "v1-0-0".into(),
            nodes: Some(vec![("n1".into(), 1), ("n2".into(), 3)]),
            logs: Some(4),
            node_id: "handler-node".into(),
            event_version: None,
            event_id: "event-1".into(),
            payload: flow_like_types::json::to_vec(&json!({ "k": 1 })).unwrap(),
            is_remote: false,
        };
        let candidate = CorpusCandidate::from_log_meta(&meta);
        assert_eq!(candidate.source_node_id, "handler-node");
        assert!(candidate.failed);
        assert_eq!(candidate.payload, json!({ "k": 1 }));
        assert_eq!(candidate.visited_node_ids, vec!["n1", "n2"]);

        let empty = LogMeta {
            payload: vec![],
            log_level: LogLevel::Warn.to_u8(),
            ..meta
        };
        let candidate = CorpusCandidate::from_log_meta(&empty);
        assert_eq!(candidate.payload, Value::Null);
        assert!(!candidate.failed);
    }

    #[test]
    fn dedupe_by_run_id_keeps_newest() {
        let rows = vec![
            candidate("r1", 100, json!({ "a": 1 }), false),
            candidate("r1", 300, json!({ "a": 2 }), false),
            candidate("r2", 200, json!({ "b": 1 }), false),
        ];
        let deduped = dedupe_by_run_id(rows);
        assert_eq!(deduped.len(), 2);
        assert_eq!(deduped[0].run_id, "r1");
        assert_eq!(deduped[0].start, 300);
        assert_eq!(deduped[1].run_id, "r2");
    }

    #[test]
    fn dedupe_by_shape_keeps_newest_per_shape() {
        let rows = vec![
            candidate("r1", 100, json!({ "a": 1 }), false),
            candidate("r2", 300, json!({ "a": 7 }), false),
            candidate("r3", 200, json!({ "b": "x" }), false),
        ];
        let deduped = dedupe_by_shape(rows);
        let run_ids: Vec<&str> = deduped.iter().map(|row| row.run_id.as_str()).collect();
        assert_eq!(run_ids, vec!["r2", "r3"]);
    }

    #[test]
    fn stratify_never_trims_failures() {
        let mut rows: Vec<CorpusCandidate> = (0..10)
            .map(|i| {
                candidate(
                    &format!("pass-{i}"),
                    1000 + i,
                    json!({ "i": i, "kind": format!("shape-{i}") }),
                    false,
                )
            })
            .collect();
        rows.push(candidate("fail-old", 1, json!({ "boom": true }), true));

        let selected = stratify_failures(rows, 5);
        assert_eq!(selected.len(), 5);
        assert!(
            selected.iter().any(|row| row.run_id == "fail-old"),
            "the oldest failing row must survive the trim"
        );
        assert_eq!(selected.last().unwrap().run_id, "fail-old");
    }

    #[test]
    fn stratify_returns_all_rows_under_target() {
        let rows = vec![
            candidate("r1", 100, json!({ "a": 1 }), false),
            candidate("r2", 200, json!({ "b": 1 }), true),
        ];
        let selected = stratify_failures(rows, 10);
        assert_eq!(selected.len(), 2);
        assert_eq!(selected[0].run_id, "r2");
    }

    #[test]
    fn refine_filters_exclusion_before_dedupe() {
        // The replay (newest, excluded) must not displace the real recording
        // of the same shape.
        let rows = vec![
            candidate("replay-1", 900, json!({ "a": 1 }), false),
            candidate("real-1", 500, json!({ "a": 2 }), false),
        ];
        let excluded: HashSet<String> = ["replay-1".to_string()].into();
        let refined = refine_corpus_rows(rows, &excluded, 10);
        assert_eq!(refined.len(), 1);
        assert_eq!(refined[0].run_id, "real-1");
    }

    #[tokio::test]
    async fn select_widens_until_target_is_met() {
        let scans: Arc<Mutex<Vec<Duration>>> = Arc::new(Mutex::new(vec![]));
        let scans_clone = scans.clone();
        let selection = select_corpus_window(2, &HashSet::new(), move |window, cap| {
            let scans = scans_clone.clone();
            async move {
                assert_eq!(cap, CORPUS_SCAN_CAP);
                scans.lock().unwrap().push(window);
                if window >= CORPUS_WINDOWS[2] {
                    Ok(vec![
                        candidate("r1", 100, json!({ "a": 1 }), false),
                        candidate("r2", 200, json!({ "b": 1 }), false),
                    ])
                } else {
                    Ok(vec![candidate("r1", 100, json!({ "a": 1 }), false)])
                }
            }
        })
        .await
        .unwrap();

        assert_eq!(selection.candidates.len(), 2);
        assert_eq!(selection.window, CORPUS_WINDOWS[2]);
        assert!(!selection.scan_capped);
        assert_eq!(
            *scans.lock().unwrap(),
            vec![CORPUS_WINDOWS[0], CORPUS_WINDOWS[1], CORPUS_WINDOWS[2]]
        );
    }

    #[tokio::test]
    async fn select_stops_widening_once_a_scan_is_capped() {
        let scans: Arc<Mutex<usize>> = Arc::new(Mutex::new(0));
        let scans_clone = scans.clone();
        let selection = select_corpus_window(400, &HashSet::new(), move |_window, cap| {
            let scans = scans_clone.clone();
            async move {
                *scans.lock().unwrap() += 1;
                // Every row shares one shape, so refinement collapses far
                // below target — but the scan is capped, so widening stops.
                Ok((0..cap)
                    .map(|i| candidate(&format!("r{i}"), i as u64, json!({ "a": i }), false))
                    .collect())
            }
        })
        .await
        .unwrap();

        assert_eq!(*scans.lock().unwrap(), 1);
        assert!(selection.scan_capped);
        assert_eq!(selection.scanned_rows, CORPUS_SCAN_CAP);
        assert_eq!(selection.window, CORPUS_WINDOWS[0]);
        assert_eq!(selection.candidates.len(), 1);
    }

    #[tokio::test]
    async fn select_returns_widest_selection_when_target_is_never_met() {
        let selection = select_corpus_window(50, &HashSet::new(), |_window, _cap| async {
            Ok(vec![candidate("r1", 100, json!({ "a": 1 }), false)])
        })
        .await
        .unwrap();

        assert_eq!(selection.candidates.len(), 1);
        assert_eq!(selection.window, *CORPUS_WINDOWS.last().unwrap());
        assert!(!selection.scan_capped);
    }
}

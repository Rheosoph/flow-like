//! Replay through an exclusive cursor. The consumer acknowledges every event it
//! receives, so a page must never carry a later sequence while an earlier one can
//! still appear: the cursor would move past it for good.

use super::types::*;
use async_trait::async_trait;
use std::{collections::BTreeMap, iter::Peekable, vec::IntoIter};

/// How long a missing sequence of a running run may still arrive, measured from
/// the event after it, which the same callback may have written. The executor
/// sends a batch at most `EXECUTOR_CALLBACK_RETRIES` (3) more times, abandons each
/// attempt after `EXECUTOR_CALLBACK_TIMEOUT_MS` (5 s) and pauses 200 ms × 2^attempt
/// plus up to half of that in between, so its last attempt starts within 17.1 s of
/// the first. The API can still be writing an abandoned attempt until
/// `POST /execution/events` hits its 120 s deadline; the remaining 12.9 s cover
/// clock skew between API instances. A longer callback timeout, more retries, a
/// batch the executor requeues after all its attempts failed, or Teams delivery
/// callbacks (130 s per attempt) let a write land after this window, behind
/// consumers already past it.
pub const UNSETTLED_GAP_MS: i64 = 150_000;

/// How far a lagging index may trail a write that landed. A sequence it still
/// omits once its write could last have landed this long ago was never written.
const INDEX_LAG_MS: i64 = 60_000;

/// The sequences one page reads consistently to settle a gap: one DynamoDB
/// BatchGetItem. A wider hole takes further polls until it lapses.
const GAP_PROBE_SEQUENCES: i32 = 100;

/// How far past its last indexed event a running run is read consistently, so a
/// wakeup that beats the index still finds the batch that caused it.
const TAIL_PROBE_SEQUENCES: i32 = 16;

/// One executor callback at the default `EXECUTOR_MAX_BATCH_SIZE`. A terminal
/// run is probed this far past its last event, so a batch the executor gave up
/// on cannot hide a final batch the index does not show yet.
const DROPPED_BATCH_SEQUENCES: i32 = 100;

/// Strongly consistent point reads of a run's canonical events.
#[async_trait]
pub trait ConsistentEventReads: Sync {
    /// The canonical events of `run_id` at `sequences`. Absent ones are left out.
    async fn read_sequences(
        &self,
        run_id: &str,
        sequences: &[i32],
    ) -> Result<Vec<ExecutionEventRecord>, StateStoreError>;
}

/// The query for one page, with one extra row to tell whether more follow.
pub fn replay_page_query(run_id: &str, cursor: ReplayCursor) -> EventQuery {
    EventQuery {
        run_id: run_id.to_owned(),
        after_sequence: Some(cursor.after_sequence),
        only_undelivered: false,
        limit: Some(i32::try_from(cursor.limit.saturating_add(1)).unwrap_or(i32::MAX)),
    }
}

/// A page from a store whose readers only ever see a prefix of a run's writes,
/// so any gap it shows is permanent and the cursor may cross it.
pub async fn replay_prefix<S: ExecutionStateStore + ?Sized>(
    store: &S,
    run_id: &str,
    cursor: ReplayCursor,
) -> Result<EventReplay, StateStoreError> {
    Ok(prefix_page(
        store.get_events(replay_page_query(run_id, cursor)).await?,
        cursor.limit,
    ))
}

/// Executors without stable event ids could store one sequence twice; the first copy wins.
fn prefix_page(raw: Vec<ExecutionEventRecord>, limit: usize) -> EventReplay {
    let has_more = raw.len() > limit;
    let mut events: Vec<ExecutionEventRecord> = Vec::with_capacity(raw.len().min(limit));
    for event in raw.into_iter().take(limit) {
        if events
            .last()
            .is_none_or(|last| event.sequence > last.sequence)
        {
            events.push(event);
        }
    }
    EventReplay { events, has_more }
}

/// A page from a store whose index of `visible` events can lag behind its
/// writes. A sequence missing from the index is read consistently; one that is
/// still absent ends the page unless it can no longer arrive.
pub async fn replay_contiguous<R: ConsistentEventReads + ?Sized>(
    reads: &R,
    run_id: &str,
    visible: Vec<ExecutionEventRecord>,
    cursor: ReplayCursor,
    now_ms: i64,
) -> Result<EventReplay, StateStoreError> {
    let more_indexed = visible.len() > cursor.limit;
    let mut visible = visible.into_iter().peekable();
    let mut page = Page::new(cursor);
    if !page
        .fill(reads, run_id, &mut visible, cursor.terminal, now_ms)
        .await?
    {
        return Ok(page.into_replay(true));
    }

    let has_more = visible.peek().is_some() || more_indexed;
    if has_more {
        return Ok(page.into_replay(true));
    }
    if !cursor.terminal {
        return page.probe_tail(reads, run_id).await;
    }
    page.confirm_end(reads, run_id).await
}

/// The first indexed event after sequences the index does not show yet.
struct Gap {
    until: i32,
    created_at: i64,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum GapState {
    /// A skipped sequence can still be written.
    Open,
    /// Every write landed, though the index may not show all of them yet.
    Settled,
    /// The index shows every write that landed, so the skipped sequences never will.
    Lapsed,
}

impl Gap {
    fn state(&self, terminal: bool, now_ms: i64) -> GapState {
        let age = now_ms.saturating_sub(self.created_at);
        if age >= UNSETTLED_GAP_MS + INDEX_LAG_MS {
            GapState::Lapsed
        } else if terminal || age >= UNSETTLED_GAP_MS {
            GapState::Settled
        } else {
            GapState::Open
        }
    }
}

struct Page {
    events: Vec<ExecutionEventRecord>,
    next: i32,
    limit: usize,
}

impl Page {
    fn new(cursor: ReplayCursor) -> Self {
        Self {
            events: Vec::new(),
            next: cursor.after_sequence.saturating_add(1),
            limit: cursor.limit,
        }
    }

    fn full(&self) -> bool {
        self.events.len() >= self.limit
    }

    fn push(&mut self, event: ExecutionEventRecord) {
        self.next = event.sequence.saturating_add(1);
        self.events.push(event);
    }

    fn into_replay(self, has_more: bool) -> EventReplay {
        EventReplay {
            events: self.events,
            has_more,
        }
    }

    /// Takes indexed events while they continue the page and stops at a gap.
    /// Executors without stable identities never promised contiguous sequences,
    /// and their duplicate sequences are dropped.
    fn take_contiguous(
        &mut self,
        visible: &mut Peekable<IntoIter<ExecutionEventRecord>>,
    ) -> Option<Gap> {
        while !self.full() {
            let upcoming = visible.peek()?;
            if upcoming.sequence > self.next && has_canonical_record_identity(upcoming) {
                return Some(Gap {
                    until: upcoming.sequence,
                    created_at: upcoming.created_at,
                });
            }
            if let Some(event) = visible.next_if(|event| event.sequence >= self.next) {
                self.push(event);
            } else {
                visible.next();
            }
        }
        None
    }

    /// Takes the indexed events and crosses the gaps between them, reading at
    /// most one consistently. `false` when the page stops at a gap.
    async fn fill<R: ConsistentEventReads + ?Sized>(
        &mut self,
        reads: &R,
        run_id: &str,
        visible: &mut Peekable<IntoIter<ExecutionEventRecord>>,
        terminal: bool,
        now_ms: i64,
    ) -> Result<bool, StateStoreError> {
        let mut probed = false;
        while let Some(gap) = self.take_contiguous(visible) {
            match gap.state(terminal, now_ms) {
                GapState::Lapsed => self.next = gap.until,
                // One consistent read per page bounds a poll however wide the hole is.
                _ if probed => return Ok(false),
                state => {
                    probed = true;
                    let settled = state == GapState::Settled;
                    if !self.fill_gap(reads, run_id, gap.until, settled).await? {
                        return Ok(false);
                    }
                }
            }
        }
        Ok(true)
    }

    /// Reads one probe's worth of the sequences before `until` that the index
    /// skipped. `false` when one is still missing and can still arrive.
    async fn fill_gap<R: ConsistentEventReads + ?Sized>(
        &mut self,
        reads: &R,
        run_id: &str,
        until: i32,
        settled: bool,
    ) -> Result<bool, StateStoreError> {
        // A missing sequence ends an open gap's page, so reading past the page's
        // room gains nothing; a settled one is crossed without taking room.
        let room = i32::try_from(self.limit - self.events.len()).unwrap_or(i32::MAX);
        let span = if settled {
            GAP_PROBE_SEQUENCES
        } else {
            room.min(GAP_PROBE_SEQUENCES)
        };
        let end = until.min(self.next.saturating_add(span));
        let missing: Vec<i32> = (self.next..end).collect();
        let mut found = by_sequence(run_id, reads.read_sequences(run_id, &missing).await?);
        for sequence in missing {
            if self.full() {
                break;
            }
            match found.remove(&sequence) {
                Some(event) => self.push(event),
                None if settled => self.next = sequence.saturating_add(1),
                None => return Ok(false),
            }
        }
        Ok(true)
    }

    /// A wakeup can arrive before the index shows the write that caused it, so a
    /// running page with room reads the next few sequences consistently and takes
    /// the contiguous prefix that exists. A miss only ends the page.
    async fn probe_tail<R: ConsistentEventReads + ?Sized>(
        mut self,
        reads: &R,
        run_id: &str,
    ) -> Result<EventReplay, StateStoreError> {
        let room = self.limit.saturating_sub(self.events.len());
        if room == 0 {
            return Ok(self.into_replay(false));
        }
        let span = i32::try_from(room)
            .unwrap_or(i32::MAX)
            .min(TAIL_PROBE_SEQUENCES);
        let window: Vec<i32> = (self.next..self.next.saturating_add(span)).collect();
        let mut found = by_sequence(run_id, reads.read_sequences(run_id, &window).await?);
        let mut appended = false;
        for sequence in window {
            match found.remove(&sequence) {
                Some(event) => {
                    self.push(event);
                    appended = true;
                }
                None => break,
            }
        }
        let has_more = appended && self.full();
        Ok(self.into_replay(has_more))
    }

    /// The terminal status landed after every event write, so a consistent miss
    /// is final, and the stream ended unless an event follows within one batch
    /// the executor may have dropped.
    async fn confirm_end<R: ConsistentEventReads + ?Sized>(
        mut self,
        reads: &R,
        run_id: &str,
    ) -> Result<EventReplay, StateStoreError> {
        let window: Vec<i32> =
            (self.next..=self.next.saturating_add(DROPPED_BATCH_SEQUENCES)).collect();
        let following = by_sequence(run_id, reads.read_sequences(run_id, &window).await?);
        let has_more = !following.is_empty();
        for event in following.into_values() {
            if self.full() {
                break;
            }
            self.push(event);
        }
        Ok(self.into_replay(has_more))
    }
}

fn by_sequence(
    run_id: &str,
    events: Vec<ExecutionEventRecord>,
) -> BTreeMap<i32, ExecutionEventRecord> {
    events
        .into_iter()
        .filter(|event| event.run_id == run_id)
        .map(|event| (event.sequence, event))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;

    const NOW: i64 = 10_000_000;

    /// The base table behind a lagging index: consistent reads see every write.
    #[derive(Default)]
    struct BaseTable {
        events: HashMap<i32, ExecutionEventRecord>,
        reads: Mutex<Vec<Vec<i32>>>,
    }

    #[async_trait]
    impl ConsistentEventReads for BaseTable {
        async fn read_sequences(
            &self,
            _run_id: &str,
            sequences: &[i32],
        ) -> Result<Vec<ExecutionEventRecord>, StateStoreError> {
            self.reads.lock().unwrap().push(sequences.to_vec());
            Ok(sequences
                .iter()
                .filter_map(|sequence| self.events.get(sequence))
                .cloned()
                .collect())
        }
    }

    fn event(sequence: i32, created_at: i64) -> ExecutionEventRecord {
        ExecutionEventRecord {
            id: canonical_execution_event_id("run-a", sequence),
            run_id: "run-a".into(),
            sequence,
            event_type: "chunk".into(),
            payload: serde_json::json!(sequence),
            delivered: false,
            expires_at: NOW + 86_400_000,
            created_at,
        }
    }

    fn legacy(sequence: i32) -> ExecutionEventRecord {
        ExecutionEventRecord {
            id: format!("random-{sequence}"),
            ..event(sequence, NOW)
        }
    }

    fn table(sequences: &[i32]) -> BaseTable {
        BaseTable {
            events: sequences
                .iter()
                .map(|sequence| (*sequence, event(*sequence, NOW)))
                .collect(),
            ..Default::default()
        }
    }

    fn cursor(after_sequence: i32, limit: usize, terminal: bool) -> ReplayCursor {
        ReplayCursor {
            after_sequence,
            limit,
            terminal,
        }
    }

    fn sequences(replay: &EventReplay) -> Vec<i32> {
        replay.events.iter().map(|event| event.sequence).collect()
    }

    async fn replay(base: &BaseTable, visible: &[i32], cursor: ReplayCursor) -> EventReplay {
        let visible = visible
            .iter()
            .map(|sequence| base.events[sequence].clone())
            .collect();
        replay_contiguous(base, "run-a", visible, cursor, NOW)
            .await
            .unwrap()
    }

    #[test]
    fn prefix_pages_keep_the_first_copy_of_a_duplicated_sequence() {
        let page = prefix_page(vec![legacy(0), legacy(1), legacy(1), legacy(2)], 3);
        assert_eq!(sequences(&page), [0, 1]);
        assert!(page.has_more);
        let page = prefix_page(vec![legacy(1), legacy(1), legacy(2)], 3);
        assert_eq!(sequences(&page), [1, 2]);
        assert!(!page.has_more);
    }

    #[tokio::test]
    async fn an_index_gap_is_filled_by_consistent_reads_before_the_cursor_moves() {
        let base = table(&[0, 1, 2, 3]);
        let page = replay(&base, &[0, 3], cursor(-1, 10, false)).await;
        assert_eq!(sequences(&page), [0, 1, 2, 3]);
        assert!(!page.has_more);
        assert_eq!(base.reads.lock().unwrap()[0], vec![1, 2]);
    }

    #[tokio::test]
    async fn a_write_still_in_flight_stops_the_page_before_its_sequence() {
        let base = table(&[0, 2, 3]);
        let page = replay(&base, &[0, 2, 3], cursor(-1, 10, false)).await;
        assert_eq!(sequences(&page), [0]);
        assert!(page.has_more);

        let page = replay(&base, &[2, 3], cursor(0, 10, false)).await;
        assert!(page.events.is_empty());
        assert!(page.has_more);
    }

    #[tokio::test]
    async fn a_gap_that_can_no_longer_fill_is_crossed() {
        let mut base = table(&[0, 2]);
        let page = replay(&base, &[0, 2], cursor(-1, 10, true)).await;
        assert_eq!(sequences(&page), [0, 2]);
        assert!(!page.has_more);

        base.events.insert(2, event(2, NOW - UNSETTLED_GAP_MS));
        let page = replay(&base, &[0, 2], cursor(-1, 10, false)).await;
        assert_eq!(sequences(&page), [0, 2]);
        assert!(!page.has_more);
        assert_eq!(base.reads.lock().unwrap()[0], vec![1]);
    }

    #[tokio::test]
    async fn a_lapsed_gap_is_crossed_without_reading_it() {
        let lapsed = NOW - UNSETTLED_GAP_MS - INDEX_LAG_MS;
        let mut base = table(&[0]);
        for sequence in [5_000, 5_001] {
            base.events.insert(sequence, event(sequence, lapsed));
        }
        let page = replay(&base, &[0, 5_000, 5_001], cursor(-1, 100, false)).await;
        assert_eq!(sequences(&page), [0, 5_000, 5_001]);
        assert!(!page.has_more);
        assert_eq!(
            base.reads.lock().unwrap().as_slice(),
            [(5_002..5_002 + TAIL_PROBE_SEQUENCES).collect::<Vec<_>>()],
            "only the tail is read"
        );
    }

    #[tokio::test]
    async fn a_wide_settled_hole_costs_one_read_per_page_until_it_lapses() {
        let far = 10 * GAP_PROBE_SEQUENCES;
        let mut base = table(&[0, far]);
        let page = replay(&base, &[0, far], cursor(-1, 10, true)).await;
        assert_eq!(sequences(&page), [0]);
        assert!(page.has_more);
        assert_eq!(
            base.reads.lock().unwrap().as_slice(),
            [(1..=GAP_PROBE_SEQUENCES).collect::<Vec<_>>()]
        );

        base.events
            .insert(far, event(far, NOW - UNSETTLED_GAP_MS - INDEX_LAG_MS));
        base.reads.lock().unwrap().clear();
        let page = replay(&base, &[far], cursor(0, 10, true)).await;
        assert_eq!(sequences(&page), [far]);
        assert!(!page.has_more);
        assert_eq!(
            base.reads.lock().unwrap().as_slice(),
            [(far + 1..=far + 1 + DROPPED_BATCH_SEQUENCES).collect::<Vec<_>>()],
            "only the end of the stream is read"
        );
    }

    #[tokio::test]
    async fn a_terminal_run_ends_only_when_nothing_follows_its_last_event() {
        let base = table(&[0, 1, 2]);
        let page = replay(&base, &[0], cursor(-1, 10, true)).await;
        assert_eq!(sequences(&page), [0, 1, 2]);
        assert!(page.has_more, "the index lagged behind the terminal status");

        let page = replay(&base, &[2], cursor(1, 10, true)).await;
        assert_eq!(sequences(&page), [2]);
        assert!(!page.has_more);
        assert_eq!(
            base.reads.lock().unwrap().last().unwrap(),
            &(3..=3 + DROPPED_BATCH_SEQUENCES).collect::<Vec<_>>()
        );

        let page = replay(&base, &[], cursor(2, 10, true)).await;
        assert!(page.events.is_empty());
        assert!(!page.has_more);
    }

    #[tokio::test]
    async fn a_dropped_batch_does_not_hide_the_final_events_of_a_terminal_run() {
        let dropped_until = 4 + DROPPED_BATCH_SEQUENCES;
        let base = table(&[0, 1, 2, 3, 4, dropped_until + 1, dropped_until + 2]);
        let page = replay(&base, &[0, 1, 2, 3, 4], cursor(-1, 200, true)).await;
        assert_eq!(sequences(&page), [0, 1, 2, 3, 4, dropped_until + 1]);
        assert!(page.has_more);

        let page = replay(&base, &[], cursor(dropped_until + 1, 200, true)).await;
        assert_eq!(sequences(&page), [dropped_until + 2]);
        assert!(page.has_more);

        let page = replay(&base, &[], cursor(dropped_until + 2, 200, true)).await;
        assert!(page.events.is_empty());
        assert!(!page.has_more);
    }

    #[tokio::test]
    async fn a_full_terminal_page_still_reports_events_after_it() {
        let base = table(&[0, 1, 2]);
        let page = replay(&base, &[0, 1], cursor(-1, 2, true)).await;
        assert_eq!(sequences(&page), [0, 1]);
        assert!(page.has_more);
    }

    #[tokio::test]
    async fn a_running_run_reads_past_a_lagging_index_for_its_tail() {
        let base = table(&[0, 1, 2]);
        let page = replay(&base, &[0], cursor(-1, 100, false)).await;
        assert_eq!(sequences(&page), [0, 1, 2]);
        assert!(!page.has_more);
        assert_eq!(
            base.reads.lock().unwrap().as_slice(),
            [(1..1 + TAIL_PROBE_SEQUENCES).collect::<Vec<_>>()]
        );

        let page = replay(&base, &[], cursor(-1, 10, false)).await;
        assert_eq!(sequences(&page), [0, 1, 2]);

        let base = table(&[0, 2]);
        let page = replay(&base, &[0], cursor(-1, 10, false)).await;
        assert_eq!(sequences(&page), [0], "the prefix stops at the first miss");
        assert!(!page.has_more);
    }

    #[tokio::test]
    async fn the_tail_probe_respects_the_page_limit() {
        let base = table(&[0, 1, 2, 3, 4]);
        let page = replay(&base, &[0], cursor(-1, 3, false)).await;
        assert_eq!(sequences(&page), [0, 1, 2]);
        assert!(page.has_more);
        assert_eq!(base.reads.lock().unwrap().as_slice(), [vec![1, 2]]);

        let page = replay(&base, &[0, 1, 2], cursor(-1, 3, false)).await;
        assert_eq!(sequences(&page), [0, 1, 2]);
        assert!(!page.has_more);
        assert_eq!(
            base.reads.lock().unwrap().len(),
            1,
            "a full page is not probed"
        );
    }

    #[tokio::test]
    async fn pages_are_bounded_and_say_when_more_are_indexed() {
        let base = table(&[0, 1, 2, 3]);
        let page = replay(&base, &[0, 1, 2], cursor(-1, 2, false)).await;
        assert_eq!(sequences(&page), [0, 1]);
        assert!(page.has_more);

        let page = replay(&base, &[0, 3], cursor(-1, 2, false)).await;
        assert_eq!(sequences(&page), [0, 1]);
        assert!(page.has_more);
        assert_eq!(base.reads.lock().unwrap().last().unwrap(), &vec![1]);

        let page = replay(&base, &[0, 3], cursor(-1, 2, true)).await;
        assert_eq!(sequences(&page), [0, 1]);
        assert!(page.has_more);
        assert_eq!(
            base.reads.lock().unwrap().last().unwrap(),
            &vec![1, 2],
            "a settled hole is read past the page's room"
        );
    }

    #[tokio::test]
    async fn legacy_events_and_duplicate_sequences_keep_their_old_delivery() {
        let base = BaseTable::default();
        let visible = vec![legacy(1), legacy(1), legacy(3)];
        let page = replay_contiguous(&base, "run-a", visible, cursor(-1, 10, false), NOW)
            .await
            .unwrap();
        assert_eq!(sequences(&page), [1, 3]);
        assert!(!page.has_more);
    }

    #[tokio::test]
    async fn consistent_reads_never_contribute_another_runs_events() {
        let mut base = table(&[0, 2]);
        base.events.insert(
            1,
            ExecutionEventRecord {
                run_id: "run-b".into(),
                ..event(1, NOW)
            },
        );
        let page = replay(&base, &[0, 2], cursor(-1, 10, false)).await;
        assert_eq!(sequences(&page), [0]);
        assert!(page.has_more);
    }
}

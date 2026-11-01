//! Replay through an exclusive cursor. The consumer acknowledges every event it
//! receives, so a page must never carry a later sequence while an earlier one can
//! still appear: the cursor would move past it for good.

use super::types::*;
use async_trait::async_trait;
use std::{collections::BTreeMap, iter::Peekable, vec::IntoIter};

/// How long a missing sequence of a running run may still arrive, measured from
/// the event after it. An executor callback that wrote that event can still be
/// writing the missing one until `POST /execution/events` hits its 120 s
/// deadline; after that the executor gave up on the batch.
pub const UNSETTLED_GAP_MS: i64 = 150_000;

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
    let mut events = store.get_events(replay_page_query(run_id, cursor)).await?;
    let has_more = events.len() > cursor.limit;
    events.truncate(cursor.limit);
    Ok(EventReplay { events, has_more })
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
    while let Some(gap) = page.take_contiguous(&mut visible) {
        let settled = gap.settled(cursor.terminal, now_ms);
        if !page.fill_gap(reads, run_id, gap.until, settled).await? {
            return Ok(page.into_replay(true));
        }
    }

    let has_more = visible.peek().is_some() || more_indexed;
    if has_more || !cursor.terminal {
        return Ok(page.into_replay(has_more));
    }
    page.confirm_end(reads, run_id).await
}

/// The first indexed event after sequences the index does not show yet.
struct Gap {
    until: i32,
    created_at: i64,
}

impl Gap {
    /// Whether the skipped sequences can no longer be written.
    fn settled(&self, terminal: bool, now_ms: i64) -> bool {
        terminal || now_ms.saturating_sub(self.created_at) >= UNSETTLED_GAP_MS
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

    /// Reads the sequences before `until` that the index skipped. `false` when
    /// one is still missing and can still arrive.
    async fn fill_gap<R: ConsistentEventReads + ?Sized>(
        &mut self,
        reads: &R,
        run_id: &str,
        until: i32,
        settled: bool,
    ) -> Result<bool, StateStoreError> {
        let room = i32::try_from(self.limit - self.events.len()).unwrap_or(i32::MAX);
        let missing: Vec<i32> = (self.next..until.min(self.next.saturating_add(room))).collect();
        let mut found = by_sequence(run_id, reads.read_sequences(run_id, &missing).await?);
        for sequence in missing {
            match found.remove(&sequence) {
                Some(event) => self.push(event),
                None if settled => self.next = sequence.saturating_add(1),
                None => return Ok(false),
            }
        }
        Ok(true)
    }

    /// The terminal status landed after every event write, so a consistent miss
    /// of the next sequence is the end of the stream.
    async fn confirm_end<R: ConsistentEventReads + ?Sized>(
        mut self,
        reads: &R,
        run_id: &str,
    ) -> Result<EventReplay, StateStoreError> {
        let next = self.next;
        let following =
            by_sequence(run_id, reads.read_sequences(run_id, &[next]).await?).remove(&next);
        let has_more = following.is_some();
        if let Some(event) = following
            && !self.full()
        {
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

    #[tokio::test]
    async fn an_index_gap_is_filled_by_consistent_reads_before_the_cursor_moves() {
        let base = table(&[0, 1, 2, 3]);
        let page = replay(&base, &[0, 3], cursor(-1, 10, false)).await;
        assert_eq!(sequences(&page), [0, 1, 2, 3]);
        assert!(!page.has_more);
        assert_eq!(base.reads.lock().unwrap().as_slice(), [vec![1, 2]]);
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
    }

    #[tokio::test]
    async fn a_terminal_run_ends_only_when_the_next_sequence_is_absent() {
        let base = table(&[0, 1, 2]);
        let page = replay(&base, &[0], cursor(-1, 10, true)).await;
        assert_eq!(sequences(&page), [0, 1]);
        assert!(page.has_more, "the index lagged behind the terminal status");

        let page = replay(&base, &[2], cursor(1, 10, true)).await;
        assert_eq!(sequences(&page), [2]);
        assert!(!page.has_more);
        assert_eq!(base.reads.lock().unwrap().last().unwrap(), &vec![3]);

        let page = replay(&base, &[], cursor(2, 10, true)).await;
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
    async fn a_running_run_does_not_probe_past_its_last_indexed_event() {
        let base = table(&[0, 1]);
        let page = replay(&base, &[0], cursor(-1, 10, false)).await;
        assert_eq!(sequences(&page), [0]);
        assert!(!page.has_more);
        assert!(base.reads.lock().unwrap().is_empty());
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
        assert!(base.reads.lock().unwrap().is_empty());
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

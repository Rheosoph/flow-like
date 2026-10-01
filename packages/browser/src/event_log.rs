// Derived from rustwright src/lib.rs @fca1438, Copyright (c) 2026 Ikonomos Inc (dba Skyvern), MIT; modified by Rheosoph GmbH. See NOTICE.
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};

use tokio::sync::watch;

use crate::error::BrowserError;
use crate::types::SessionId;

const DEFAULT_MAX_EVENTS: usize = 8192;
const DEFAULT_MAX_BYTES: usize = 8 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct Event {
    pub seq: u64,
    pub method: std::sync::Arc<str>,
    pub session: Option<SessionId>,
    pub params: std::sync::Arc<serde_json::Value>,
    pub received: tokio::time::Instant,
}

impl Event {
    pub fn decode<T: serde::de::DeserializeOwned>(&self) -> Option<T> {
        match T::deserialize(&*self.params) {
            Ok(value) => Some(value),
            Err(error) => {
                tracing::debug!(method = %self.method, %error, "typed event decode failed");
                None
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EventCursor(pub u64);

#[derive(Clone, Copy, Debug)]
pub struct EventLogLimits {
    pub max_events: usize,
    pub max_bytes: usize,
}

impl Default for EventLogLimits {
    fn default() -> Self {
        Self {
            max_events: DEFAULT_MAX_EVENTS,
            max_bytes: DEFAULT_MAX_BYTES,
        }
    }
}

struct Retained {
    event: Event,
    bytes: usize,
}

#[derive(Default)]
struct LogState {
    events: VecDeque<Retained>,
    bytes: usize,
    newest_evicted: Option<u64>,
    closed: Option<String>,
}

pub struct EventLog {
    limits: EventLogLimits,
    next_seq: AtomicU64,
    state: Mutex<LogState>,
    changed: watch::Sender<u64>,
}

impl EventLog {
    pub(crate) fn new(limits: EventLogLimits) -> Self {
        Self {
            limits,
            next_seq: AtomicU64::new(1),
            state: Mutex::new(LogState::default()),
            changed: watch::Sender::new(0),
        }
    }

    pub fn cursor(&self) -> EventCursor {
        EventCursor(self.next_seq.load(Ordering::Acquire))
    }

    pub(crate) fn next_seq(&self) -> u64 {
        self.next_seq.fetch_add(1, Ordering::AcqRel)
    }

    pub(crate) fn push(&self, event: Event) {
        let bytes = event.params.to_string().len() + event.method.len();
        {
            let mut state = self.state();
            state.bytes += bytes;
            state.events.push_back(Retained { event, bytes });
            while state.events.len() > self.limits.max_events
                || (state.bytes > self.limits.max_bytes && state.events.len() > 1)
            {
                let Some(evicted) = state.events.pop_front() else {
                    break;
                };
                state.bytes -= evicted.bytes;
                state.newest_evicted = Some(evicted.event.seq);
            }
        }
        self.changed.send_modify(|version| *version += 1);
    }

    pub(crate) fn close(&self, reason: &str) {
        self.state().closed.get_or_insert_with(|| reason.to_owned());
        self.changed.send_modify(|version| *version += 1);
    }

    pub async fn wait_for(
        &self,
        cursor: EventCursor,
        deadline: tokio::time::Instant,
        filter: impl Fn(&Event) -> bool,
    ) -> crate::Result<Option<Event>> {
        let mut changed = self.changed.subscribe();
        let mut from = cursor.0;
        loop {
            if let Some(found) = self.scan(&mut from, &filter)? {
                return Ok(Some(found));
            }
            if tokio::time::timeout_at(deadline, changed.changed())
                .await
                .is_err()
            {
                return self.scan(&mut from, &filter);
            }
        }
    }

    fn scan(
        &self,
        from: &mut u64,
        filter: &impl Fn(&Event) -> bool,
    ) -> crate::Result<Option<Event>> {
        let state = self.state();
        if state.newest_evicted.is_some_and(|evicted| evicted >= *from) {
            return Err(BrowserError::EventsLost);
        }
        let found = state
            .events
            .iter()
            .map(|retained| &retained.event)
            .filter(|event| event.seq >= *from)
            .find(|event| filter(event))
            .cloned();
        if found.is_none() {
            if let Some(reason) = &state.closed {
                return Err(BrowserError::Disconnected {
                    reason: reason.clone(),
                });
            }
            if let Some(newest) = state.events.back() {
                *from = (*from).max(newest.event.seq + 1);
            }
        }
        Ok(found)
    }

    fn state(&self) -> MutexGuard<'_, LogState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::time::Duration;

    fn event(log: &EventLog, method: &str) -> Event {
        Event {
            seq: log.next_seq(),
            method: method.into(),
            session: None,
            params: Arc::new(serde_json::json!({"size": "x".repeat(10)})),
            received: tokio::time::Instant::now(),
        }
    }

    #[tokio::test]
    async fn wait_for_finds_events_after_the_cursor() {
        let log = EventLog::new(EventLogLimits::default());
        log.push(event(&log, "Page.frameNavigated"));
        let cursor = log.cursor();
        log.push(event(&log, "Page.frameNavigated"));
        let deadline = tokio::time::Instant::now() + Duration::from_millis(50);
        let found = log
            .wait_for(cursor, deadline, |event| {
                &*event.method == "Page.frameNavigated"
            })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(found.seq, cursor.0);
        let missing = log
            .wait_for(log.cursor(), deadline, |_| true)
            .await
            .unwrap();
        assert!(missing.is_none());
    }

    #[tokio::test]
    async fn lagging_cursors_report_lost_events() {
        let log = EventLog::new(EventLogLimits {
            max_events: 2,
            max_bytes: usize::MAX,
        });
        let cursor = log.cursor();
        for _ in 0..3 {
            log.push(event(&log, "Target.targetCreated"));
        }
        let deadline = tokio::time::Instant::now() + Duration::from_millis(10);
        assert!(matches!(
            log.wait_for(cursor, deadline, |_| false).await,
            Err(BrowserError::EventsLost)
        ));
    }

    #[tokio::test]
    async fn bytes_bound_the_log_but_keep_the_newest_event() {
        let one = event(&EventLog::new(EventLogLimits::default()), "Page.x");
        let size = one.params.to_string().len() + one.method.len();
        let log = EventLog::new(EventLogLimits {
            max_events: usize::MAX,
            max_bytes: size * 2,
        });
        let first = log.cursor();
        for _ in 0..3 {
            log.push(event(&log, "Page.x"));
        }
        let deadline = tokio::time::Instant::now();
        assert!(matches!(
            log.wait_for(first, deadline, |_| false).await,
            Err(BrowserError::EventsLost)
        ));
        let retained = log
            .wait_for(EventCursor(first.0 + 1), deadline, |_| true)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(retained.seq, first.0 + 1);
        let tiny = EventLog::new(EventLogLimits {
            max_events: 8,
            max_bytes: 1,
        });
        tiny.push(event(&tiny, "Page.big"));
        let kept = tiny
            .wait_for(EventCursor(1), deadline, |_| true)
            .await
            .unwrap();
        assert!(kept.is_some());
    }

    #[tokio::test]
    async fn waiters_that_kept_up_survive_later_evictions() {
        let log = Arc::new(EventLog::new(EventLogLimits {
            max_events: 2,
            max_bytes: usize::MAX,
        }));
        let waiter = tokio::spawn({
            let log = log.clone();
            async move {
                let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
                log.wait_for(log.cursor(), deadline, |event| {
                    &*event.method == "Target.targetDestroyed"
                })
                .await
            }
        });
        tokio::task::yield_now().await;
        for _ in 0..2 {
            log.push(event(&log, "Target.targetInfoChanged"));
            tokio::task::yield_now().await;
        }
        log.push(event(&log, "Target.targetDestroyed"));
        let found = waiter.await.unwrap().unwrap().unwrap();
        assert_eq!(&*found.method, "Target.targetDestroyed");
    }

    #[tokio::test]
    async fn closing_wakes_waiters_with_the_reason() {
        let log = Arc::new(EventLog::new(EventLogLimits::default()));
        let waiter = tokio::spawn({
            let log = log.clone();
            async move {
                let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
                log.wait_for(log.cursor(), deadline, |_| true).await
            }
        });
        tokio::task::yield_now().await;
        log.close("eof");
        assert!(matches!(
            waiter.await.unwrap(),
            Err(BrowserError::Disconnected { reason }) if reason == "eof"
        ));
    }
}

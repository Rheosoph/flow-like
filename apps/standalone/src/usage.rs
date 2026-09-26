use serde::{Deserialize, Serialize};
use std::{
    sync::{Arc, LazyLock, Mutex},
    time::Instant,
};

/// Cumulative counters for this process's supervised services and native hosts.
/// Payload sizes exclude HTTP/SSE framing. No payload contents are retained.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeUsageSnapshot {
    pub version: u32,
    pub sequence: u64,
    pub process_uptime_ms: u64,
    pub invocations_started: u64,
    pub invocations_succeeded: u64,
    pub invocations_failed: u64,
    pub invocations_cancelled: u64,
    pub in_flight: u64,
    pub runtime_messages: u64,
    pub request_payload_bytes: u64,
    pub response_payload_bytes: u64,
    pub concurrency_rejections: u64,
}

impl RuntimeUsageSnapshot {
    pub fn validate_after(&self, previous: Option<&Self>) -> bool {
        let completed = self
            .invocations_succeeded
            .checked_add(self.invocations_failed)
            .and_then(|value| value.checked_add(self.invocations_cancelled))
            .and_then(|value| value.checked_add(self.in_flight));
        self.version == 1
            && self.sequence > 0
            && completed == Some(self.invocations_started)
            && previous.is_none_or(|old| {
                self.sequence > old.sequence
                    && self.process_uptime_ms >= old.process_uptime_ms
                    && self.invocations_started >= old.invocations_started
                    && self.invocations_succeeded >= old.invocations_succeeded
                    && self.invocations_failed >= old.invocations_failed
                    && self.invocations_cancelled >= old.invocations_cancelled
                    && self.runtime_messages >= old.runtime_messages
                    && self.request_payload_bytes >= old.request_payload_bytes
                    && self.response_payload_bytes >= old.response_payload_bytes
                    && self.concurrency_rejections >= old.concurrency_rejections
            })
    }
}

struct Counters {
    started: Instant,
    value: Mutex<RuntimeUsageSnapshot>,
}
impl Counters {
    fn new() -> Self {
        Self {
            started: Instant::now(),
            value: Mutex::new(RuntimeUsageSnapshot {
                version: 1,
                ..Default::default()
            }),
        }
    }
    fn snapshot(&self) -> RuntimeUsageSnapshot {
        let mut value = self.value.lock().unwrap_or_else(|error| error.into_inner());
        value.sequence = value.sequence.saturating_add(1);
        value.process_uptime_ms = self.started.elapsed().as_millis().min(u64::MAX as u128) as u64;
        value.clone()
    }
    fn begin(self: &Arc<Self>, request_bytes: u64) -> Invocation {
        let mut value = self.value.lock().unwrap_or_else(|error| error.into_inner());
        value.invocations_started = value.invocations_started.saturating_add(1);
        value.in_flight = value.in_flight.saturating_add(1);
        value.request_payload_bytes = value.request_payload_bytes.saturating_add(request_bytes);
        Invocation {
            counters: self.clone(),
            complete: false,
        }
    }
}

static PROCESS: LazyLock<Arc<Counters>> = LazyLock::new(|| Arc::new(Counters::new()));

#[cfg(feature = "runtime")]
pub(crate) fn service_observer()
-> Arc<dyn flow_like_runtime::flow::execution::service::ServiceObserver> {
    Arc::new(ServiceCounters(PROCESS.clone()))
}

#[cfg(feature = "runtime")]
struct ServiceCounters(Arc<Counters>);

#[cfg(feature = "runtime")]
impl flow_like_runtime::flow::execution::service::ServiceObserver for ServiceCounters {
    fn begin(
        &self,
        request_bytes: u64,
    ) -> Box<dyn flow_like_runtime::flow::execution::service::ServiceInvocation> {
        Box::new(self.0.begin(request_bytes))
    }
    fn message(&self) {
        let mut value = self
            .0
            .value
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        value.runtime_messages = value.runtime_messages.saturating_add(1);
    }
    fn response_bytes(&self, bytes: usize) {
        let mut value = self
            .0
            .value
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        value.response_payload_bytes = value.response_payload_bytes.saturating_add(bytes as u64);
    }
}

#[cfg(feature = "runtime")]
impl flow_like_runtime::flow::execution::service::ServiceInvocation for Invocation {
    fn finish(
        mut self: Box<Self>,
        outcome: flow_like_runtime::flow::execution::service::ServiceOutcome,
    ) {
        use flow_like_runtime::flow::execution::service::ServiceOutcome;
        self.record(match outcome {
            ServiceOutcome::Succeeded => Outcome::Succeeded,
            ServiceOutcome::Failed => Outcome::Failed,
            ServiceOutcome::Cancelled => Outcome::Cancelled,
        });
    }
}

pub fn snapshot() -> RuntimeUsageSnapshot {
    PROCESS.snapshot()
}

pub(crate) fn begin(request_bytes: u64) -> Invocation {
    PROCESS.begin(request_bytes)
}

pub(crate) fn runtime_message() {
    let mut value = PROCESS
        .value
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    value.runtime_messages = value.runtime_messages.saturating_add(1);
}

pub(crate) fn response_payload(bytes: usize) {
    let mut value = PROCESS
        .value
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    value.response_payload_bytes = value.response_payload_bytes.saturating_add(bytes as u64);
}

pub(crate) fn concurrency_rejected() {
    let mut value = PROCESS
        .value
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    value.concurrency_rejections = value.concurrency_rejections.saturating_add(1);
}

pub(crate) enum Outcome {
    Succeeded,
    Failed,
    Cancelled,
}

pub(crate) struct Invocation {
    counters: Arc<Counters>,
    complete: bool,
}
impl Invocation {
    pub fn finish(mut self, outcome: Outcome) {
        self.record(outcome);
    }
    fn record(&mut self, outcome: Outcome) {
        if self.complete {
            return;
        }
        self.complete = true;
        let mut value = self
            .counters
            .value
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        value.in_flight = value.in_flight.saturating_sub(1);
        let counter = match outcome {
            Outcome::Succeeded => &mut value.invocations_succeeded,
            Outcome::Failed => &mut value.invocations_failed,
            Outcome::Cancelled => &mut value.invocations_cancelled,
        };
        *counter = counter.saturating_add(1);
    }
}
impl Drop for Invocation {
    fn drop(&mut self) {
        self.record(Outcome::Cancelled);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "runtime")]
    #[test]
    fn workflow_observer_records_completion_and_dropped_requests_once() {
        use flow_like_runtime::flow::execution::service::{ServiceObserver, ServiceOutcome};
        let counters = Arc::new(Counters::new());
        let observer = ServiceCounters(counters.clone());
        observer.begin(10).finish(ServiceOutcome::Succeeded);
        observer.begin(20).finish(ServiceOutcome::Failed);
        observer.begin(30).finish(ServiceOutcome::Cancelled);
        drop(observer.begin(40));
        observer.message();
        observer.response_bytes(15);
        let value = counters.snapshot();
        assert!(value.validate_after(None));
        assert_eq!(value.invocations_started, 4);
        assert_eq!(value.invocations_cancelled, 2);
        assert_eq!(value.in_flight, 0);
        assert_eq!(value.runtime_messages, 1);
        assert_eq!(value.request_payload_bytes, 100);
        assert_eq!(value.response_payload_bytes, 15);
    }
    #[test]
    fn snapshots_distinguish_completion_failure_and_cancelled_work() {
        let counters = Arc::new(Counters::new());
        let first = counters.snapshot();
        assert!(first.validate_after(None));
        let running = counters.begin(20);
        counters.begin(10).finish(Outcome::Succeeded);
        counters.begin(30).finish(Outcome::Failed);
        let active = counters.snapshot();
        assert!(active.validate_after(Some(&first)));
        assert_eq!(active.in_flight, 1);
        assert_eq!(active.request_payload_bytes, 60);
        drop(running);
        let final_snapshot = counters.snapshot();
        assert!(final_snapshot.validate_after(Some(&active)));
        assert_eq!(final_snapshot.invocations_cancelled, 1);
        assert_eq!(final_snapshot.in_flight, 0);
        assert!(!active.validate_after(Some(&final_snapshot)));
        let mut invalid = final_snapshot.clone();
        invalid.invocations_started += 1;
        assert!(!invalid.validate_after(None));
    }
}

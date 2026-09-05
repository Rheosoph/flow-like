use super::log::LogMessage;
use crate::flow::variable::Variable;
use ahash::AHashMap;
use flow_like_types::sync::Mutex;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::{fmt::Write, sync::Arc, time::SystemTime};

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
pub struct Trace {
    pub id: String,
    pub node_id: Arc<str>,
    pub logs: Vec<LogMessage>,
    pub start: SystemTime,
    pub end: SystemTime,

    // for debugging purposes only
    pub variables: Option<Vec<Variable>>,
}

/// Monotonic source for trace ids.
///
/// A trace is created for every `ExecutionContext`, i.e. once per node execution, so this
/// runs tens of millions of times on a loop-heavy board. `cuid2` is SHA3-based and was
/// costing far more than the identifier is worth: nothing joins on a trace id — log rows
/// carry their own `operation_id` and are grouped by `node_id` — so a counter is enough.
static TRACE_SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

impl Trace {
    pub fn new(node_id: &str) -> Self {
        Self::new_shared(Arc::from(node_id))
    }

    pub fn new_shared(node_id: Arc<str>) -> Self {
        Trace {
            id: Self::next_id(),
            node_id,
            logs: vec![],
            start: SystemTime::now(),
            end: SystemTime::now(),
            variables: None,
        }
    }

    /// Process-unique trace id. Runs are stored in per-run tables, so uniqueness within a
    /// process is all any consumer can rely on anyway.
    fn next_id() -> String {
        let sequence = TRACE_SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let mut id = String::with_capacity(21);
        write!(&mut id, "t{sequence}").expect("writing to a String cannot fail");
        id
    }

    /// Move the trace payload out while preserving its identity on the context.
    ///
    /// Contexts may be merged more than once by nested execution paths. Moving the
    /// logs makes subsequent merges contribute only logs written after the first merge.
    pub fn take(&mut self) -> Self {
        Self {
            id: self.id.clone(),
            node_id: self.node_id.clone(),
            logs: std::mem::take(&mut self.logs),
            start: self.start,
            end: self.end,
            variables: self.variables.take(),
        }
    }

    pub fn get_start(&self) -> SystemTime {
        if self.logs.is_empty() {
            return self.start;
        }

        let found_earliest = self.logs.iter().min_by_key(|log| log.start).unwrap();
        found_earliest.start
    }

    pub fn finish(&mut self) {
        self.end = SystemTime::now();
    }

    pub async fn snapshot_variables(&mut self, variables: &Arc<Mutex<AHashMap<String, Variable>>>) {
        self.variables = Some(variables.lock().await.values().cloned().collect());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow::execution::LogLevel;

    #[test]
    fn trace_ids_are_unique() {
        let first = Trace::new("node");
        let second = Trace::new("node");

        assert_ne!(first.id, second.id);
    }

    #[test]
    fn take_moves_logs_and_preserves_identity() {
        let mut trace = Trace::new("node");
        trace
            .logs
            .push(LogMessage::new("message", LogLevel::Info, None));
        let id = trace.id.clone();
        let node_id = trace.node_id.clone();

        let taken = trace.take();

        assert_eq!(taken.id, id);
        assert_eq!(taken.node_id, node_id);
        assert_eq!(taken.logs.len(), 1);
        assert!(trace.logs.is_empty());
        assert_eq!(trace.id, id);
        assert_eq!(trace.node_id, node_id);
    }
}

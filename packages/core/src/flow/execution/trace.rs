use super::log::LogMessage;
use crate::flow::variable::Variable;
use ahash::AHashMap;
use flow_like_types::sync::Mutex;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::{sync::Arc, time::SystemTime};

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
pub struct Trace {
    pub id: String,
    pub node_id: String,
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
        let now = SystemTime::now();
        Trace {
            id: Self::next_id(),
            node_id: node_id.to_string(),
            logs: vec![],
            // One clock read: `end` is overwritten by `finish()` before anything reads it,
            // and this runs once per node execution.
            start: now,
            end: now,
            variables: None,
        }
    }

    /// Process-unique trace id. Runs are stored in per-run tables, so uniqueness within a
    /// process is all any consumer can rely on anyway.
    fn next_id() -> String {
        use std::fmt::Write;
        let sequence = TRACE_SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let mut id = String::with_capacity(17);
        // `write!` formats straight into the buffer; `to_string()` would allocate a
        // throwaway String first.
        let _ = write!(id, "t{sequence}");
        id
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

    /// Move the accumulated contents out, leaving an empty trace with the same identity.
    ///
    /// Handing logs upward used to clone them. In a loop that is O(iterations × depth)
    /// duplicated `LogMessage` strings for data the child drops immediately afterwards.
    /// The identity is kept rather than moved so that anything logged after this point is
    /// still attributed to the same node.
    pub fn take(&mut self) -> Trace {
        Trace {
            id: self.id.clone(),
            node_id: self.node_id.clone(),
            logs: std::mem::take(&mut self.logs),
            start: self.start,
            end: self.end,
            variables: self.variables.take(),
        }
    }

    pub async fn snapshot_variables(&mut self, variables: &Arc<Mutex<AHashMap<String, Variable>>>) {
        self.variables = Some(variables.lock().await.values().cloned().collect());
    }
}

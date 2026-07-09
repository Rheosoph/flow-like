//! Process-mining correlation carried across execution & app-connection JWTs.
//!
//! A run's causal tree is reconstructable from `ExecutionRun.parent_run_id`
//! alone, but to avoid walking that chain on every query (and to inherit
//! business keys across app hops) we denormalize the tree root (`trace_id`)
//! and the business/object keys onto every run — propagated through the same
//! JWT rails that already carry `run_id` and `app_chain`.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Business keys ride inside JWTs across every hop, so they are hard-capped:
/// an unbounded map would bloat the Authorization header past proxy limits.
pub const MAX_CORRELATION_KEYS: usize = 8;
pub const MAX_CORRELATION_KEY_LEN: usize = 64;
pub const MAX_CORRELATION_VALUE_LEN: usize = 256;

/// Validates caller-supplied business keys before they are stamped on a run
/// and propagated through signed tokens.
pub fn validate_business_keys(keys: &HashMap<String, String>) -> Result<(), String> {
    if keys.len() > MAX_CORRELATION_KEYS {
        return Err(format!(
            "At most {MAX_CORRELATION_KEYS} correlation keys are allowed per run"
        ));
    }
    for (key, value) in keys {
        if key.is_empty() || key.len() > MAX_CORRELATION_KEY_LEN {
            return Err(format!(
                "Correlation key '{key}' must be 1-{MAX_CORRELATION_KEY_LEN} characters"
            ));
        }
        if value.len() > MAX_CORRELATION_VALUE_LEN {
            return Err(format!(
                "Correlation value for '{key}' exceeds {MAX_CORRELATION_VALUE_LEN} characters"
            ));
        }
    }
    Ok(())
}

/// Denormalized process-correlation context propagated with a run.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct CorrelationContext {
    /// Root run id of the causal tree (shared by every run in the tree).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace_id: Option<String>,
    /// Business/object keys (order_id, customer_id, …) tagging the case.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub keys: HashMap<String, String>,
}

impl CorrelationContext {
    /// Context for a run that roots a new trace (no caller to inherit from).
    pub fn root(run_id: &str) -> Self {
        Self {
            trace_id: Some(run_id.to_string()),
            keys: HashMap::new(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.trace_id.is_none() && self.keys.is_empty()
    }

    /// `Some(self)` unless empty — for `skip_serializing_if` friendliness when
    /// storing an `Option<CorrelationContext>` on claims/params.
    pub fn into_option(self) -> Option<Self> {
        if self.is_empty() { None } else { Some(self) }
    }

    /// Merge extra keys in (extra wins on conflict), keeping `trace_id`.
    pub fn with_keys(mut self, extra: &HashMap<String, String>) -> Self {
        for (key, value) in extra {
            self.keys.insert(key.clone(), value.clone());
        }
        self
    }

    /// The keys as a JSON object for the `correlationKeys` column, or `None`.
    pub fn keys_json(&self) -> Option<serde_json::Value> {
        if self.keys.is_empty() {
            None
        } else {
            serde_json::to_value(&self.keys).ok()
        }
    }
}

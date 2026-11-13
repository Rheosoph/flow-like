//! Bounded model usage retained with bot replies. Only structured usage frames are collected;
//! prompt text, tool arguments and provider response bodies never enter the saved record.

use flow_like_model_provider::response::LLMUsageStats;
use flow_like_types::bot_message::{MAX_USAGE_STATS_BYTES, MAX_USAGE_STATS_RECORDS};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

use super::run_recovery::BotRunRecovery;

#[derive(Clone, Debug, Default)]
pub struct BotUsage(Arc<Mutex<BTreeMap<String, LLMUsageStats>>>);

#[derive(Deserialize, Serialize)]
struct Record {
    step_name: String,
    stats: LLMUsageStats,
}

impl BotUsage {
    /// Usage frames are cumulative snapshots. A final or replayed snapshot replaces its prior
    /// value so resuming delivery never counts the same model work twice.
    pub fn observe(&self, frame: &str) {
        if frame.starts_with('{') {
            let Ok(event) = serde_json::from_str::<Value>(frame) else {
                return;
            };
            if event["type"] == "native_specialist_event"
                && event["toolName"] == "spawn_subtasks"
                && let (Some(id), Some(frame)) = (
                    event["usageId"]
                        .as_str()
                        .or_else(|| event["requestId"].as_str()),
                    event["frame"].as_str(),
                )
                && let Some(payload) = frame
                    .strip_prefix("<usage_stat>")
                    .and_then(|value| value.strip_suffix("</usage_stat>"))
                && let Ok(record) = serde_json::from_str::<Record>(payload)
            {
                self.record(
                    &format!("subtask:{}", blake3::hash(id.as_bytes()).to_hex()),
                    record.stats,
                );
            }
            return;
        }
        let Some(payload) = frame
            .strip_prefix("<usage_stat>")
            .and_then(|value| value.strip_suffix("</usage_stat>"))
        else {
            return;
        };
        let Ok(record) = serde_json::from_str::<Record>(payload) else {
            return;
        };
        self.record(&record.step_name, record.stats);
    }

    pub fn record(&self, step_name: &str, mut stats: LLMUsageStats) {
        let name: String = step_name.chars().take(80).collect();
        if let Some(model) = &mut stats.model {
            *model = model.chars().take(128).collect();
        }
        for call in &mut stats.calls {
            call.model = call.model.chars().take(128).collect();
            if let Some(purpose) = &mut call.purpose {
                *purpose = purpose.chars().take(64).collect();
            }
        }
        for event in &mut stats.compactions {
            event.kind = event.kind.chars().take(64).collect();
            event.trigger = event.trigger.chars().take(64).collect();
            event.status = event.status.chars().take(64).collect();
            if let Some(model) = &mut event.model {
                *model = model.chars().take(128).collect();
            }
        }
        let mut records = self
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        records.insert(name, stats);
    }

    pub fn restore(&self, checkpoint: &flow_like_types::bot_turn::BotRunCheckpoint) {
        if let Some(records) = checkpoint
            .state
            .get("usage_stats")
            .and_then(Value::as_array)
        {
            for record in records {
                if let Ok(record) = serde_json::from_value::<Record>(record.clone()) {
                    self.record(&record.step_name, record.stats);
                }
            }
        }
        if let Some(stats) = checkpoint.state.get("stats")
            && let Ok(stats) = serde_json::from_value(stats.clone())
        {
            self.record("Assistant", stats);
        }
    }

    pub fn with_recovery(&self, inner: Arc<dyn BotRunRecovery>) -> Arc<dyn BotRunRecovery> {
        Arc::new(UsageRecovery {
            inner,
            usage: self.clone(),
            restored: AtomicBool::new(false),
        })
    }

    pub fn snapshot(&self) -> Vec<Value> {
        let records = self
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut grouped = BTreeMap::<String, LLMUsageStats>::new();
        for (name, stats) in records.iter() {
            let name = if name.starts_with("subtask:") {
                "Delegated model work"
            } else {
                name
            };
            match grouped.entry(name.into()) {
                std::collections::btree_map::Entry::Vacant(entry) => {
                    entry.insert(stats.clone());
                }
                std::collections::btree_map::Entry::Occupied(mut entry) => {
                    entry.get_mut().merge(stats);
                }
            }
        }
        if grouped.len() > MAX_USAGE_STATS_RECORDS {
            let mut other = LLMUsageStats::default();
            grouped.retain(|name, stats| {
                if name == "Assistant" {
                    return true;
                }
                other.merge(stats);
                false
            });
            grouped.insert("Other model work".into(), other);
        }
        let mut values: Vec<Value> = grouped
            .iter()
            .filter_map(|(name, stats)| {
                serde_json::to_value(Record {
                    step_name: name.clone(),
                    stats: stats.clone(),
                })
                .ok()
            })
            .collect();
        for record in &mut values {
            for field in ["calls", "compactions"] {
                if let Some(rows) = record
                    .get_mut("stats")
                    .and_then(|stats| stats.get_mut(field))
                    .and_then(Value::as_array_mut)
                {
                    let omitted = rows.len().saturating_sub(16);
                    rows.drain(..omitted);
                    if omitted > 0 {
                        let key = format!("{field}_omitted");
                        record["stats"][&key] = json!(
                            record["stats"][&key]
                                .as_u64()
                                .unwrap_or(0)
                                .saturating_add(omitted as u64)
                        );
                    }
                }
            }
        }
        // Keep the most recent detail rows; complete token/cache totals stay in `usage`.
        while serde_json::to_vec(&values).map_or(usize::MAX, |bytes| bytes.len())
            > MAX_USAGE_STATS_BYTES
        {
            let mut removed = false;
            for field in ["calls", "compactions"] {
                if let Some(record) = values
                    .iter_mut()
                    .max_by_key(|record| record["stats"][field].as_array().map_or(0, Vec::len))
                    && let Some(rows) = record
                        .get_mut("stats")
                        .and_then(|stats| stats.get_mut(field))
                        .and_then(Value::as_array_mut)
                    && !rows.is_empty()
                {
                    rows.remove(0);
                    let omitted = format!("{field}_omitted");
                    record["stats"][&omitted] =
                        json!(record["stats"][&omitted].as_u64().unwrap_or(0) + 1);
                    removed = true;
                    break;
                }
            }
            if !removed {
                break;
            }
        }
        values
    }

    pub fn attach(&self, body: &mut Value) {
        attach_usage_stats(body, self.snapshot());
    }
}

/// Leave the same body budget that tool parts use, including room for sealed envelopes. When a
/// reply already fills it, metadata yields to its text; no caller text is shortened here.
pub fn attach_usage_stats(body: &mut Value, mut stats: Vec<Value>) {
    if stats.is_empty() || !body.is_object() {
        return;
    }
    let used = serde_json::to_vec(body).map_or(usize::MAX, |bytes| bytes.len());
    let budget = flow_like_types::bot_tool_trail::MAX_BODY_BYTES
        .saturating_sub(used)
        .saturating_sub(20);
    while !stats.is_empty()
        && serde_json::to_vec(&stats).map_or(usize::MAX, |bytes| bytes.len()) > budget
    {
        let mut removed = false;
        for field in ["calls", "compactions"] {
            if let Some(record) = stats
                .iter_mut()
                .max_by_key(|record| record["stats"][field].as_array().map_or(0, Vec::len))
                && let Some(rows) = record
                    .get_mut("stats")
                    .and_then(|stats| stats.get_mut(field))
                    .and_then(Value::as_array_mut)
                && !rows.is_empty()
            {
                rows.remove(0);
                let key = format!("{field}_omitted");
                record["stats"][&key] = json!(
                    record["stats"][&key]
                        .as_u64()
                        .unwrap_or(0)
                        .saturating_add(1)
                );
                removed = true;
                break;
            }
        }
        if !removed {
            stats.pop();
        }
    }
    if !stats.is_empty() {
        body["usage_stats"] = json!(stats);
    }
}

#[derive(Debug)]
struct UsageRecovery {
    inner: Arc<dyn BotRunRecovery>,
    usage: BotUsage,
    restored: AtomicBool,
}

#[async_trait::async_trait]
impl BotRunRecovery for UsageRecovery {
    async fn load(
        &self,
    ) -> flow_like_types::Result<Option<flow_like_types::bot_turn::BotRunCheckpoint>> {
        let checkpoint = self.inner.load().await?;
        if !self.restored.swap(true, Ordering::AcqRel)
            && let Some(checkpoint) = &checkpoint
        {
            self.usage.restore(checkpoint);
        }
        Ok(checkpoint)
    }

    async fn save(
        &self,
        checkpoint: &flow_like_types::bot_turn::BotRunCheckpoint,
    ) -> flow_like_types::Result<()> {
        if let Some(stats) = checkpoint.state.get("stats")
            && let Ok(stats) = serde_json::from_value(stats.clone())
        {
            self.usage.record("Assistant", stats);
        }
        let mut checkpoint = checkpoint.clone();
        checkpoint.state["usage_stats"] = json!(self.usage.snapshot());
        self.inner.save(&checkpoint).await
    }

    fn deadline_at_ms(&self) -> Option<i64> {
        self.inner.deadline_at_ms()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_model_provider::response::{CompactionStats, ModelCallEntry, Usage};

    #[test]
    fn repeated_snapshots_keep_totals_and_bound_saved_details() {
        let usage = BotUsage::default();
        let stats = LLMUsageStats {
            usage: Usage {
                prompt_tokens: 100_000,
                completion_tokens: 1_000,
                total_tokens: 101_000,
                ..Default::default()
            },
            calls: (0..500)
                .map(|_| ModelCallEntry {
                    model: "model".repeat(30),
                    ..Default::default()
                })
                .collect(),
            compactions: (0..100)
                .map(|_| CompactionStats {
                    kind: "memory".into(),
                    status: "completed".into(),
                    ..Default::default()
                })
                .collect(),
            calls_omitted: 9,
            ..Default::default()
        };
        let frame = super::super::stream::usage_stat_frame("Assistant", &json!(stats));
        usage.observe(&frame);
        usage.observe(&frame);
        let snapshot = usage.snapshot();
        assert_eq!(snapshot.len(), 1);
        assert_eq!(snapshot[0]["stats"]["usage"]["total_tokens"], 101_000);
        assert!(snapshot[0]["stats"]["calls_omitted"].as_u64().unwrap() >= 9 + 484);
        assert!(serde_json::to_vec(&snapshot).unwrap().len() <= MAX_USAGE_STATS_BYTES);
        let mut body = json!({"text": "x".repeat(48_000)});
        usage.attach(&mut body);
        assert!(
            serde_json::to_vec(&flow_like_types::inbox::plain_envelope(body))
                .unwrap()
                .len()
                < 65_536
        );
        let text = "界".repeat(13_000);
        let mut body = json!({"text": text});
        usage.attach(&mut body);
        assert_eq!(body["text"], text);
        assert_eq!(
            body["usage_stats"][0]["stats"]["usage"]["total_tokens"],
            101_000
        );
        assert!(
            serde_json::to_vec(&body).unwrap().len()
                <= flow_like_types::bot_tool_trail::MAX_BODY_BYTES
        );
        let mut full = json!({"text": "x".repeat(60_000)});
        let before = full.clone();
        usage.attach(&mut full);
        assert_eq!(full, before);
    }

    #[test]
    fn unstructured_text_and_unknown_payload_fields_are_not_saved() {
        let usage = BotUsage::default();
        usage.observe("model text <usage_stat>{}</usage_stat>");
        usage.observe("<usage_stat>{bad json}</usage_stat>");
        assert!(usage.snapshot().is_empty());
        usage.observe(&format!("<usage_stat>{}</usage_stat>", json!({
            "step_name": "Assistant", "prompt": "private prompt", "stats": {
                "usage": {"prompt_tokens": 1, "completion_tokens": 2, "total_tokens": 3, "cost": null},
                "prompt": "private prompt"
            }
        })));
        assert!(
            !json!(usage.snapshot())
                .to_string()
                .contains("private prompt")
        );
        assert_eq!(usage.snapshot()[0]["stats"]["usage"]["total_tokens"], 3);
    }

    #[test]
    fn delegated_snapshots_have_independent_identity_and_preserve_parent_usage() {
        let usage = BotUsage::default();
        let stats = LLMUsageStats {
            usage: Usage {
                total_tokens: 7,
                ..Default::default()
            },
            ..Default::default()
        };
        usage.record("Assistant", stats.clone());
        let child = |id| {
            json!({
                "type": "native_specialist_event", "toolName": "spawn_subtasks", "requestId": id,
                "frame": super::super::stream::usage_stat_frame("Assistant", &json!(stats)),
            })
            .to_string()
        };
        usage.observe(&child("one"));
        usage.observe(&child("two"));
        usage.observe(&child("one"));
        let snapshot = usage.snapshot();
        assert_eq!(
            snapshot
                .iter()
                .map(|record| record["stats"]["usage"]["total_tokens"].as_u64().unwrap())
                .sum::<u64>(),
            21
        );
        assert_eq!(snapshot.len(), 2);
        assert_eq!(snapshot[0]["step_name"], "Assistant");
        assert_eq!(snapshot[0]["stats"]["usage"]["total_tokens"], 7);
    }

    #[test]
    fn delivery_restore_recovers_checkpoint_usage_without_prompt_content() {
        let usage = BotUsage::default();
        let checkpoint = flow_like_types::bot_turn::BotRunCheckpoint {
            revision: 1,
            started_at_ms: 1,
            deadline_at_ms: 2,
            state: json!({"original_request": "private", "stats": LLMUsageStats {
                usage: Usage { total_tokens: 42, ..Default::default() }, ..Default::default()
            }}),
        };
        usage.restore(&checkpoint);
        usage.restore(&checkpoint);
        assert_eq!(usage.snapshot()[0]["stats"]["usage"]["total_tokens"], 42);
        assert!(!json!(usage.snapshot()).to_string().contains("private"));
    }

    #[derive(Debug, Default)]
    struct Saved(Mutex<Option<flow_like_types::bot_turn::BotRunCheckpoint>>);

    #[async_trait::async_trait]
    impl BotRunRecovery for Saved {
        async fn load(
            &self,
        ) -> flow_like_types::Result<Option<flow_like_types::bot_turn::BotRunCheckpoint>> {
            Ok(self.0.lock().unwrap().clone())
        }
        async fn save(
            &self,
            checkpoint: &flow_like_types::bot_turn::BotRunCheckpoint,
        ) -> flow_like_types::Result<()> {
            *self.0.lock().unwrap() = Some(checkpoint.clone());
            Ok(())
        }
        fn deadline_at_ms(&self) -> Option<i64> {
            Some(100)
        }
    }

    #[tokio::test]
    async fn checkpoints_keep_delegated_usage_without_counting_repeated_loads_twice() {
        let stats = |tokens| LLMUsageStats {
            usage: Usage {
                total_tokens: tokens,
                ..Default::default()
            },
            ..Default::default()
        };
        let checkpoint = flow_like_types::bot_turn::BotRunCheckpoint {
            revision: 1,
            started_at_ms: 1,
            deadline_at_ms: 100,
            state: json!({"stats": stats(7)}),
        };
        let storage = Arc::new(Saved::default());
        let first = BotUsage::default();
        first.record("subtask:one", stats(11));
        first
            .with_recovery(storage.clone())
            .save(&checkpoint)
            .await
            .unwrap();
        let saved = storage.load().await.unwrap().unwrap();
        for record in saved.state["usage_stats"].as_array().unwrap() {
            assert!(record["stats"].get("calls").is_none());
            assert!(record["stats"].get("compactions").is_none());
            serde_json::from_value::<Record>(record.clone())
                .expect("bounded checkpoint records still deserialize");
        }
        let resumed = BotUsage::default();
        let recovery = resumed.with_recovery(storage);
        recovery.load().await.unwrap();
        resumed.record("subtask:two", stats(13));
        recovery.load().await.unwrap();
        recovery.save(&checkpoint).await.unwrap();
        let stored = recovery.load().await.unwrap().unwrap();
        let total: u64 = stored.state["usage_stats"]
            .as_array()
            .unwrap()
            .iter()
            .map(|record| record["stats"]["usage"]["total_tokens"].as_u64().unwrap())
            .sum();
        assert_eq!(total, 31);
        assert_eq!(recovery.deadline_at_ms(), Some(100));
    }
}

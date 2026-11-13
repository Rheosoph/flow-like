//! Usage belongs to the memory operation that made the call, even when turns overlap.

use std::future::Future;
use std::sync::{Arc, Mutex};

use flow_like_model_provider::response::{LLMUsageStats, Usage};

use crate::engine::MemoryEngine;
use crate::llm::LlmTask;

flow_like_types::tokio::task_local! {
    static ACTIVE: Arc<Mutex<LLMUsageStats>>;
}

pub(crate) async fn capture<T>(work: impl Future<Output = T>) -> (T, LLMUsageStats) {
    let stats = Arc::new(Mutex::new(LLMUsageStats::default()));
    let mut cancellation = CancelledCapture {
        stats: stats.clone(),
        parent: ACTIVE.try_with(Clone::clone).ok(),
        completed: false,
    };
    let result = ACTIVE.scope(stats.clone(), work).await;
    cancellation.completed = true;
    let measured = std::mem::take(&mut *stats.lock().unwrap_or_else(|e| e.into_inner()));
    (result, measured)
}

struct CancelledCapture {
    stats: Arc<Mutex<LLMUsageStats>>,
    parent: Option<Arc<Mutex<LLMUsageStats>>>,
    completed: bool,
}

impl Drop for CancelledCapture {
    fn drop(&mut self) {
        if !self.completed
            && let Some(parent) = &self.parent
        {
            let stats = std::mem::take(&mut *self.stats.lock().unwrap_or_else(|e| e.into_inner()));
            parent
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .merge(&stats);
        }
    }
}

pub(crate) async fn capture_into<T>(
    stats: Arc<Mutex<LLMUsageStats>>,
    work: impl Future<Output = T>,
) -> T {
    ACTIVE.scope(stats, work).await
}

/// Owns memory measurements until the bot's usage collector accepts them. Dropping an
/// undelivered preparation returns its measurements to the conversation's pending record.
#[derive(Clone)]
pub struct UsageDelivery(Arc<Delivery>);

struct Delivery {
    engine: MemoryEngine,
    stats: Arc<Mutex<LLMUsageStats>>,
}

impl std::fmt::Debug for UsageDelivery {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("UsageDelivery")
            .finish_non_exhaustive()
    }
}

impl UsageDelivery {
    pub(crate) fn new(engine: MemoryEngine, stats: LLMUsageStats) -> Self {
        Self(Arc::new(Delivery {
            engine,
            stats: Arc::new(Mutex::new(stats)),
        }))
    }

    pub(crate) fn buffer(&self) -> Arc<Mutex<LLMUsageStats>> {
        self.0.stats.clone()
    }

    pub fn snapshot(&self) -> LLMUsageStats {
        self.0
            .stats
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Transfers the measurements once, shared by every clone of this delivery.
    pub fn take(&self) -> LLMUsageStats {
        std::mem::take(&mut *self.0.stats.lock().unwrap_or_else(|e| e.into_inner()))
    }

    pub(crate) fn merge(&self, stats: &LLMUsageStats) {
        self.0
            .stats
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .merge(stats);
    }
}

impl Drop for Delivery {
    fn drop(&mut self) {
        let stats = std::mem::take(&mut *self.stats.lock().unwrap_or_else(|e| e.into_inner()));
        let _ = self.engine.schedule_pending_usage(stats);
    }
}

pub(crate) fn record(stats: &LLMUsageStats) {
    let _ = ACTIVE.try_with(|active| {
        active
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .merge(stats);
    });
}

/// Records an observed provider response for the current memory operation. Custom model
/// adapters may call this too; adapters without provider usage leave it unknown.
pub fn record_model_usage(task: LlmTask, model: &str, usage: &Usage, duration_ms: u64) {
    let mut stats = LLMUsageStats::default();
    stats.accumulate(usage, Some(model));
    if let Some(call) = stats.calls.last_mut() {
        call.duration_ms = Some(duration_ms);
        call.purpose = Some(
            match task {
                LlmTask::SegmentSummary => "memory_segment_summary",
                LlmTask::MergeSummaries => "memory_merge_summaries",
                LlmTask::NowBlock => "memory_now_block",
                LlmTask::FactExtraction => "memory_fact_extraction",
                LlmTask::FactUpdate => "memory_fact_update",
            }
            .to_owned(),
        );
    }
    record(&stats);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn overlapping_operations_keep_their_own_usage() {
        let run = |model: &'static str, tokens| async move {
            capture(async move {
                flow_like_types::tokio::task::yield_now().await;
                record_model_usage(
                    LlmTask::SegmentSummary,
                    model,
                    &Usage {
                        prompt_tokens: tokens,
                        total_tokens: tokens,
                        ..Default::default()
                    },
                    7,
                );
            })
            .await
            .1
        };
        let (first, second) = flow_like_types::tokio::join!(run("first", 11), run("second", 23));
        assert_eq!(first.usage.prompt_tokens, 11);
        assert_eq!(second.usage.prompt_tokens, 23);
        assert_eq!(first.calls[0].model, "first");
        assert_eq!(second.calls[0].model, "second");
        assert_eq!(
            first.calls[0].purpose.as_deref(),
            Some("memory_segment_summary")
        );
    }

    #[tokio::test]
    async fn cancelled_nested_work_preserves_usage_in_the_delivery_scope() {
        let stats = Arc::new(Mutex::new(LLMUsageStats::default()));
        capture_into(stats.clone(), async {
            let work = capture(async {
                record_model_usage(
                    LlmTask::SegmentSummary,
                    "model",
                    &Usage {
                        total_tokens: 17,
                        ..Default::default()
                    },
                    1,
                );
                std::future::pending::<()>().await;
            });
            assert!(
                flow_like_types::tokio::time::timeout(std::time::Duration::from_millis(1), work)
                    .await
                    .is_err()
            );
        })
        .await;
        assert_eq!(stats.lock().unwrap().usage.total_tokens, 17);
    }
}

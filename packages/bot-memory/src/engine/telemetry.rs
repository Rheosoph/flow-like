use super::*;
use crate::telemetry::UsageDelivery;
use flow_like_model_provider::response::LLMUsageStats;

const MAX_PENDING_CALLS: usize = 128;
const MAX_PENDING_COMPACTIONS: usize = 32;
const MAX_USAGE_BATCHES: usize = 256;

impl MemoryEngine {
    /// Persist background work until a later turn can include it in its usage frame.
    pub(crate) async fn record_pending_usage(&self, stats: &LLMUsageStats) -> Result<()> {
        match self.schedule_pending_usage(stats.clone()) {
            Some(first_attempt) => first_attempt.await?,
            None => Ok(()),
        }
    }

    /// The owned task finishes the transfer even if the waiting preparation is cancelled.
    /// Its unused result then drops the guard and refunds the measurements.
    pub(crate) async fn take_pending_usage(&self) -> Result<UsageDelivery> {
        let engine = self.clone();
        flow_like_types::tokio::spawn(async move {
            let stats = engine
                .update_pending_usage(None, WaitDuration::ZERO)
                .await?;
            Ok(UsageDelivery::new(engine, stats))
        })
        .await?
    }

    pub(crate) fn schedule_pending_usage(
        &self,
        stats: LLMUsageStats,
    ) -> Option<flow_like_types::tokio::sync::oneshot::Receiver<Result<()>>> {
        if stats.calls.is_empty()
            && stats.compactions.is_empty()
            && stats.usage.total_tokens == 0
            && stats.usage.cost.is_none()
        {
            return None;
        }
        let Ok(runtime) = flow_like_types::tokio::runtime::Handle::try_current() else {
            tracing::warn!(
                tokens = stats.usage.total_tokens,
                "Memory usage could not be returned after the runtime stopped"
            );
            return None;
        };
        let engine = self.clone();
        let batch = create_id();
        let (sender, receiver) = flow_like_types::tokio::sync::oneshot::channel();
        runtime.spawn(async move {
            let mut first = Some(sender);
            for attempt in 0..CLAIM_ATTEMPTS {
                let wait = if attempt == 0 {
                    WaitDuration::ZERO
                } else {
                    WRITE_LEASE_WAIT
                };
                let result = engine
                    .update_pending_usage(Some((&batch, &stats)), wait)
                    .await;
                if let Some(first) = first.take() {
                    let report = result
                        .as_ref()
                        .map(|_| ())
                        .map_err(|error| flow_like_types::anyhow!("{error:#}"));
                    let _ = first.send(report);
                }
                if result.is_ok() {
                    return;
                }
                if attempt + 1 < CLAIM_ATTEMPTS {
                    sleep(WRITE_LEASE_POLL).await;
                } else if let Err(error) = result {
                    tracing::warn!(
                        tokens = stats.usage.total_tokens,
                        "Memory usage could not be persisted after retries: {error:#}"
                    );
                }
            }
        });
        Some(receiver)
    }

    async fn update_pending_usage(
        &self,
        incoming: Option<(&str, &LLMUsageStats)>,
        max_wait: WaitDuration,
    ) -> Result<LLMUsageStats> {
        for _ in 0..CLAIM_ATTEMPTS {
            let state = self.stores.state.load().await?;
            if incoming.is_none() && state.pending_usage.is_none() {
                return Ok(LLMUsageStats::default());
            }
            if incoming
                .is_some_and(|(batch, _)| state.usage_batches.iter().any(|saved| saved == batch))
            {
                return Ok(LLMUsageStats::default());
            }
            let incoming = incoming.map(|(batch, stats)| (batch.to_owned(), stats.clone()));
            let outcome = self
                .commit_with_wait(
                    state.revision,
                    Utc::now(),
                    max_wait,
                    move |_, mut state| async move {
                        let mut pending: LLMUsageStats = state
                            .pending_usage
                            .take()
                            .map(serde_json::from_value)
                            .transpose()?
                            .unwrap_or_default();
                        if let Some((batch, incoming)) = incoming {
                            pending.merge(&incoming);
                            // Retain totals even when a dormant conversation accumulates more detail
                            // than the next usage frame should carry.
                            let extra = pending.calls.len().saturating_sub(MAX_PENDING_CALLS);
                            pending.calls.drain(..extra);
                            pending.calls_omitted = pending
                                .calls_omitted
                                .saturating_add(extra.min(u32::MAX as usize) as u32);
                            let extra = pending
                                .compactions
                                .len()
                                .saturating_sub(MAX_PENDING_COMPACTIONS);
                            pending.compactions.drain(..extra);
                            pending.compactions_omitted = pending
                                .compactions_omitted
                                .saturating_add(extra.min(u32::MAX as usize) as u32);
                            state.pending_usage = Some(serde_json::to_value(pending)?);
                            state.usage_batches.push(batch);
                            let extra = state.usage_batches.len().saturating_sub(MAX_USAGE_BATCHES);
                            state.usage_batches.drain(..extra);
                            Ok((LLMUsageStats::default(), state))
                        } else {
                            Ok((pending, state))
                        }
                    },
                )
                .await?;
            if let CommitOutcome::Applied(stats, _) = outcome {
                return Ok(stats);
            }
        }
        flow_like_types::bail!("Memory usage changed while being recorded; retry on the next turn")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{HashEmbedder, StubLlm};
    use flow_like_model_provider::response::Usage;

    async fn engine() -> (tempfile::TempDir, MemoryEngine) {
        let directory = tempfile::tempdir().unwrap();
        let stores =
            MemoryStores::open_local(directory.path(), Arc::new(HashEmbedder::new("hash", 16)))
                .await
                .unwrap();
        let engine = MemoryEngine::new(
            stores,
            Arc::new(StubLlm::new()),
            CompactionConfig::default(),
            Tz::UTC,
        );
        (directory, engine)
    }

    fn measured() -> LLMUsageStats {
        let mut stats = LLMUsageStats::default();
        stats.record_call(
            &Usage {
                total_tokens: 17,
                ..Default::default()
            },
            "model",
            "memory_segment_summary",
            1,
        );
        stats
    }

    #[tokio::test]
    async fn cancelling_a_waiting_transfer_refunds_the_committed_usage_once() {
        let (_directory, engine) = engine().await;
        engine.record_pending_usage(&measured()).await.unwrap();
        let work = engine.work.lock().await;
        let pending_engine = engine.clone();
        let (started, ready) = flow_like_types::tokio::sync::oneshot::channel();
        let waiting = flow_like_types::tokio::spawn(async move {
            let _ = started.send(());
            pending_engine.take_pending_usage().await
        });
        ready.await.unwrap();
        waiting.abort();
        let _ = waiting.await;
        drop(work);
        flow_like_types::tokio::time::timeout(WaitDuration::from_secs(5), async {
            loop {
                let state = engine.stores.state.load().await.unwrap();
                if state.usage_batches.len() == 2 {
                    let pending: LLMUsageStats =
                        serde_json::from_value(state.pending_usage.unwrap()).unwrap();
                    assert_eq!(pending.usage.total_tokens, 17);
                    break;
                }
                sleep(WaitDuration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        let delivered = engine.take_pending_usage().await.unwrap();
        assert_eq!(delivered.take().usage.total_tokens, 17);
        assert_eq!(delivered.take().usage.total_tokens, 0);
        assert!(
            engine
                .stores
                .state
                .load()
                .await
                .unwrap()
                .pending_usage
                .is_none()
        );
    }

    #[tokio::test]
    async fn retrying_a_recorded_batch_after_delivery_does_not_replay_it() {
        let (_directory, engine) = engine().await;
        let stats = measured();
        engine
            .update_pending_usage(Some(("batch", &stats)), WaitDuration::ZERO)
            .await
            .unwrap();
        assert_eq!(
            engine
                .take_pending_usage()
                .await
                .unwrap()
                .take()
                .usage
                .total_tokens,
            17
        );
        engine
            .update_pending_usage(Some(("batch", &stats)), WaitDuration::ZERO)
            .await
            .unwrap();
        assert!(
            engine
                .stores
                .state
                .load()
                .await
                .unwrap()
                .pending_usage
                .is_none()
        );
    }
}

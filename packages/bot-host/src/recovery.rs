//! Checkpoints fenced by the hub's turn lease and claim attempt.

use crate::hub::TurnHub;

/// A current claim may allow less time than the turn's original persisted budget.
#[derive(Debug)]
pub struct CappedRecovery {
    inner: Arc<dyn BotRunRecovery>,
    deadline_at_ms: i64,
}

impl CappedRecovery {
    pub fn new(inner: Arc<dyn BotRunRecovery>, deadline_at_ms: i64) -> Self {
        Self {
            inner,
            deadline_at_ms,
        }
    }
}

#[async_trait]
impl BotRunRecovery for CappedRecovery {
    async fn load(&self) -> Result<Option<BotRunCheckpoint>> {
        self.inner.load().await
    }
    async fn save(&self, checkpoint: &BotRunCheckpoint) -> Result<()> {
        self.inner.save(checkpoint).await
    }
    fn deadline_at_ms(&self) -> Option<i64> {
        Some(
            self.inner
                .deadline_at_ms()
                .unwrap_or(self.deadline_at_ms)
                .min(self.deadline_at_ms),
        )
    }
}
use async_trait::async_trait;
use flow_like::flow::copilot::run_recovery::BotRunRecovery;
use flow_like_types::{
    Result, anyhow,
    bot_turn::{BotRunCheckpoint, BotTurn},
};
use std::sync::{
    Arc,
    atomic::{AtomicI64, Ordering},
};

pub struct HubRecovery {
    hub: Arc<dyn TurnHub>,
    bot_id: String,
    turn_id: String,
    host_id: String,
    attempt: u32,
    deadline: AtomicI64,
    encryption: Option<(Arc<crate::e2e::E2eHost>, String)>,
    sealed_cache: tokio::sync::Mutex<Option<(BotRunCheckpoint, BotRunCheckpoint)>>,
}

impl std::fmt::Debug for HubRecovery {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HubRecovery")
            .field("turn_id", &self.turn_id)
            .field("attempt", &self.attempt)
            .finish()
    }
}

impl HubRecovery {
    pub fn new(hub: Arc<dyn TurnHub>, turn: &BotTurn, host_id: &str, deadline_at_ms: i64) -> Self {
        Self {
            hub,
            bot_id: turn.bot_id.clone(),
            turn_id: turn.id.clone(),
            host_id: host_id.into(),
            attempt: turn.attempts,
            deadline: AtomicI64::new(deadline_at_ms),
            encryption: None,
            sealed_cache: Default::default(),
        }
    }

    pub fn with_encryption(
        mut self,
        host: Arc<crate::e2e::E2eHost>,
        conversation_id: String,
    ) -> Self {
        self.encryption = Some((host, conversation_id));
        self
    }
}

#[async_trait]
impl BotRunRecovery for HubRecovery {
    async fn load(&self) -> Result<Option<BotRunCheckpoint>> {
        let checkpoint = self
            .hub
            .checkpoint_load(&self.bot_id, &self.turn_id, &self.host_id, self.attempt)
            .await
            .map_err(|error| anyhow!(error.to_string()))?;
        let checkpoint = match (&self.encryption, checkpoint) {
            (Some((host, room)), Some(checkpoint)) => Some(
                host.open_checkpoint(&self.bot_id, room, &self.turn_id, &checkpoint)
                    .await
                    .map_err(|error| anyhow!(error.message))?,
            ),
            (_, checkpoint) => checkpoint,
        };
        if let Some(checkpoint) = &checkpoint {
            checkpoint.validate().map_err(|error| anyhow!(error))?;
            self.deadline
                .store(checkpoint.deadline_at_ms, Ordering::SeqCst);
        }
        Ok(checkpoint)
    }

    async fn save(&self, checkpoint: &BotRunCheckpoint) -> Result<()> {
        checkpoint.validate().map_err(|error| anyhow!(error))?;
        let stored = if let Some((host, room)) = &self.encryption {
            let mut cache = self.sealed_cache.lock().await;
            if let Some((original, sealed)) = &*cache
                && original == checkpoint
            {
                sealed.clone()
            } else {
                let sealed = host
                    .seal_checkpoint(&self.bot_id, room, &self.turn_id, checkpoint)
                    .await
                    .map_err(|error| anyhow!(error.message))?;
                *cache = Some((checkpoint.clone(), sealed.clone()));
                sealed
            }
        } else {
            checkpoint.clone()
        };
        self.hub
            .checkpoint_save(
                &self.bot_id,
                &self.turn_id,
                &self.host_id,
                self.attempt,
                &stored,
            )
            .await
            .map_err(|error| anyhow!(error.to_string()))?;
        self.deadline
            .store(checkpoint.deadline_at_ms, Ordering::SeqCst);
        Ok(())
    }

    fn deadline_at_ms(&self) -> Option<i64> {
        Some(self.deadline.load(Ordering::SeqCst))
    }
}

/// A parked approval proves this one call did not execute; leave every unknown sibling intact.
pub async fn preserve_unexecuted_approval(
    recovery: Option<&Arc<dyn BotRunRecovery>>,
    gate: &crate::gate::ApprovalGate,
) -> Result<()> {
    let (Some(recovery), Some((id, tool))) = (recovery, gate.unexecuted_call()) else {
        return Ok(());
    };
    if let Some(checkpoint) = recovery.load().await?
        && let Some(updated) = flow_like::flow::copilot::run_recovery::record_unexecuted_approval(
            &checkpoint,
            &id,
            &tool,
        )?
    {
        recovery.save(&updated).await?;
    }
    Ok(())
}

pub fn remaining_deadline(
    recovery: Option<&Arc<dyn BotRunRecovery>>,
    fallback: std::time::Duration,
) -> std::time::Duration {
    recovery
        .and_then(|recovery| recovery.deadline_at_ms())
        .map_or(fallback, |deadline| {
            // The runtime forbids new model/tool work past the stored deadline. Leave time to
            // load and publish its cached answer or static recovery handoff.
            let left = deadline.saturating_sub(chrono::Utc::now().timestamp_millis());
            if left <= 0 {
                std::time::Duration::from_secs(30).min(fallback)
            } else {
                std::time::Duration::from_millis(left as u64).min(fallback)
            }
        })
}

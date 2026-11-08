//! The local inbox's durable execution state, scoped to one bot and trigger message.

use super::store::HostStore;
use async_trait::async_trait;
use flow_like::flow::copilot::run_recovery::BotRunRecovery;
use flow_like_types::{Result, anyhow, bot_turn::BotRunCheckpoint};
use std::sync::{
    Arc,
    atomic::{AtomicI64, Ordering},
};

pub(super) struct LocalRecovery {
    store: Arc<dyn HostStore>,
    bot: String,
    room: String,
    trigger: String,
    deadline: AtomicI64,
}

impl std::fmt::Debug for LocalRecovery {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LocalRecovery")
            .field("bot", &self.bot)
            .field("room", &self.room)
            .field("trigger", &self.trigger)
            .finish()
    }
}

impl LocalRecovery {
    pub(super) fn new(
        store: Arc<dyn HostStore>,
        bot: &str,
        room: &str,
        trigger: &str,
        deadline_at_ms: i64,
    ) -> Self {
        Self {
            store,
            bot: bot.into(),
            room: room.into(),
            trigger: trigger.into(),
            deadline: AtomicI64::new(deadline_at_ms),
        }
    }
}

#[async_trait]
impl BotRunRecovery for LocalRecovery {
    async fn load(&self) -> Result<Option<BotRunCheckpoint>> {
        let checkpoint = self
            .store
            .run_checkpoint(&self.bot, &self.room, &self.trigger)
            .await
            .map_err(|error| anyhow!(error))?;
        if let Some(checkpoint) = &checkpoint {
            checkpoint.validate().map_err(|error| anyhow!(error))?;
            self.deadline
                .store(checkpoint.deadline_at_ms, Ordering::SeqCst);
        }
        Ok(checkpoint)
    }
    async fn save(&self, checkpoint: &BotRunCheckpoint) -> Result<()> {
        self.store
            .save_run_checkpoint(&self.bot, &self.room, &self.trigger, checkpoint.clone())
            .await
            .map_err(|error| anyhow!(error))?;
        self.deadline
            .store(checkpoint.deadline_at_ms, Ordering::SeqCst);
        Ok(())
    }
    fn deadline_at_ms(&self) -> Option<i64> {
        Some(self.deadline.load(Ordering::SeqCst))
    }
}

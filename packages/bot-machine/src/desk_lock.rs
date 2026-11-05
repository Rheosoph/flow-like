use std::sync::Mutex;
use std::time::Duration;

use flow_like_bot_tools::error::{ToolError, code};
use tokio::sync::Notify;
use tokio::time::Instant;

pub const WAIT_FOR_DESK: Duration = Duration::from_secs(10 * 60);
pub const DESK_LEASE: Duration = Duration::from_secs(90);

struct Holder {
    bot: String,
    last_use: Instant,
}

/// The logged-in screen is one desk: one bot at a time, queued. A bot keeps the desk while it
/// keeps using it; after `lease` of silence the next bot in line gets it.
pub struct DesktopLock {
    holder: Mutex<Option<Holder>>,
    freed: Notify,
    wait: Duration,
    lease: Duration,
}

impl Default for DesktopLock {
    fn default() -> Self {
        Self::new(WAIT_FOR_DESK, DESK_LEASE)
    }
}

impl DesktopLock {
    pub fn new(wait: Duration, lease: Duration) -> Self {
        Self {
            holder: Mutex::new(None),
            freed: Notify::new(),
            wait,
            lease,
        }
    }

    fn try_take(&self, bot: &str) -> bool {
        let mut holder = self.holder.lock().unwrap_or_else(|e| e.into_inner());
        let free = holder
            .as_ref()
            .is_none_or(|h| h.bot == bot || h.last_use.elapsed() >= self.lease);
        if free {
            *holder = Some(Holder {
                bot: bot.to_string(),
                last_use: Instant::now(),
            });
        }
        free
    }

    pub async fn acquire(&self, bot: &str) -> Result<(), ToolError> {
        let deadline = Instant::now() + self.wait;
        loop {
            if self.try_take(bot) {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(ToolError::new(
                    code::DISPLAY_BUSY,
                    format!(
                        "Another bot is using the computer; {bot} waited {} s for it. Try again later",
                        self.wait.as_secs()
                    ),
                ));
            }
            let tick = tokio::time::sleep(Duration::from_secs(1));
            tokio::select! {
                _ = self.freed.notified() => {}
                _ = tick => {}
            }
        }
    }

    pub fn release(&self, bot: &str) {
        let mut holder = self.holder.lock().unwrap_or_else(|e| e.into_inner());
        if holder.as_ref().is_some_and(|h| h.bot == bot) {
            *holder = None;
            self.freed.notify_waiters();
        }
    }

    pub fn holder(&self) -> Option<String> {
        self.holder
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .map(|h| h.bot.clone())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    #[tokio::test(start_paused = true)]
    async fn the_desk_serves_one_bot_and_the_next_waits_for_a_release() {
        let desk = Arc::new(DesktopLock::default());
        desk.acquire("a").await.unwrap();
        desk.acquire("a").await.unwrap();
        let waiting = {
            let desk = desk.clone();
            tokio::spawn(async move { desk.acquire("b").await })
        };
        tokio::time::sleep(Duration::from_secs(5)).await;
        assert!(!waiting.is_finished());
        desk.release("a");
        waiting.await.unwrap().unwrap();
        assert_eq!(desk.holder().as_deref(), Some("b"));
    }

    #[tokio::test(start_paused = true)]
    async fn a_waiting_bot_gets_a_clear_error_after_the_timeout() {
        let desk = DesktopLock::new(Duration::from_secs(30), Duration::from_secs(600));
        desk.acquire("a").await.unwrap();
        let error = desk.acquire("b").await.unwrap_err();
        assert_eq!(error.code, code::DISPLAY_BUSY);
        assert!(error.message.contains("Another bot is using the computer"));
        assert_eq!(desk.holder().as_deref(), Some("a"));
    }

    #[tokio::test(start_paused = true)]
    async fn an_idle_holder_loses_the_desk_after_its_lease() {
        let desk = DesktopLock::new(Duration::from_secs(600), Duration::from_secs(90));
        desk.acquire("a").await.unwrap();
        tokio::time::sleep(Duration::from_secs(91)).await;
        desk.acquire("b").await.unwrap();
        assert_eq!(desk.holder().as_deref(), Some("b"));
    }

    #[tokio::test(start_paused = true)]
    async fn only_the_holder_can_release() {
        let desk = DesktopLock::default();
        desk.acquire("a").await.unwrap();
        desk.release("b");
        assert_eq!(desk.holder().as_deref(), Some("a"));
    }
}

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use flow_like_bot_tools::browser::BrowserBackend;
use flow_like_bot_tools::computer::ComputerBackend;
use flow_like_types::Result;

use crate::frames::Shot;
use crate::input::InputEvent;

/// One bot's browsers (per conversation) and computer, wherever they run: in this process or
/// in the bot's worker process.
#[async_trait]
pub trait Playground: Send + Sync {
    fn browser(&self, conversation: Option<&str>) -> Arc<dyn BrowserBackend>;

    fn computer(&self) -> Arc<dyn ComputerBackend>;

    /// A picture of what the bot is looking at, without starting anything; `None` when the
    /// bot has no browser open and has not used the computer.
    async fn capture(&self) -> Result<Option<Shot>>;

    /// The owner's input on the bot's screen; `width` x `height` is the size of the frame the
    /// owner looks at.
    async fn inject(&self, event: &InputEvent, width: u32, height: u32) -> Result<()>;

    /// Ends the browsers and any input in flight.
    async fn stop(&self);

    /// Ends whatever has been idle for longer than `idle`.
    async fn sweep(&self, idle: Duration);
}

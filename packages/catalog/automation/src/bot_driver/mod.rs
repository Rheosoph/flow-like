//! The browser and the computer as bot tools use them: the node logic (snapshot refs, element
//! actions, navigation policy, desktop gestures) without a flow run around it.
#[cfg(feature = "execute")]
mod browser;
#[cfg(feature = "execute")]
mod computer;

#[cfg(feature = "execute")]
pub use browser::{BotBrowser, BotBrowserOptions, BotDownload, BotScroll, BotSnapshot, BotStep};
#[cfg(feature = "execute")]
pub use computer::{BotComputer, BotPointerButton, BotScreen, BotScrollDirection};

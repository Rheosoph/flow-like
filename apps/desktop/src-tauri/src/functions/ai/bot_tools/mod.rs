//! The browser and the computer of a bot on this device (`browser_*`, `computer_*`): the nodes'
//! logic behind `flow-like-bot-tools`' backends, plus what the bot panel's Computer tab shows.

pub mod view;
pub(crate) mod mail;
pub(crate) mod schedule;

#[cfg(any(target_os = "macos", windows, target_os = "linux"))]
mod browser;
#[cfg(any(target_os = "macos", windows, target_os = "linux"))]
mod computer;

use std::sync::Arc;

use flow_like_bot_tools::{BrowserBackend, ComputerBackend};
use tauri::AppHandle;

/// The backends one tool call may use; both `None` on phones and tablets.
#[derive(Default, Clone)]
pub(crate) struct LocalMachine {
    pub(crate) browser: Option<Arc<dyn BrowserBackend>>,
    pub(crate) computer: Option<Arc<dyn ComputerBackend>>,
}

#[cfg(any(target_os = "macos", windows, target_os = "linux"))]
pub(crate) fn local_machine(
    app: &AppHandle,
    bot_id: &str,
    conversation_id: Option<&str>,
    browser_use: bool,
    computer_use: bool,
) -> LocalMachine {
    LocalMachine {
        browser: browser_use
            .then(|| browser::session(app, bot_id, conversation_id) as Arc<dyn BrowserBackend>),
        computer: computer_use.then(|| computer::session(bot_id) as Arc<dyn ComputerBackend>),
    }
}

#[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
pub(crate) fn local_machine(
    _app: &AppHandle,
    _bot_id: &str,
    _conversation_id: Option<&str>,
    _browser_use: bool,
    _computer_use: bool,
) -> LocalMachine {
    LocalMachine::default()
}

//! The desktop's bot host: runs a local bot's turn without a window when a message waits for it
//! and posts the reply as the bot.
//!
//! - Trigger: the local store's change notifier (`UiNotifier::inbox_changed` pokes the host) and a
//!   scan of every conversation at startup. See [`rule::waiting`] for what counts as waiting.
//! - Run: `global_chat_native` with the bot's id and no webview, so the bot gets this desktop's
//!   native tools and nothing asks the person. The run id is the turn id (`bt_...`); a window that
//!   wants tokens calls `global_chat_resume(run_id = turn id)` while the turn is `running`. A run
//!   without a webview leaves the registry the moment it finishes, so a late window reads the
//!   reply from the inbox instead.
//! - IPC: `bot-host://turn` carries a [`TurnInfo`] whenever a turn changes state; the commands
//!   `bot_host_turns` and `bot_host_cancel_turn` inspect and cancel.
//! - Mobile: the host lives in the app process. While the OS suspends the app no turn runs and
//!   the bot is offline; the startup scan answers what waited, once the app runs again.

mod account;
mod approvals;
mod authorization;
pub(crate) mod commands;
pub(crate) mod connector_commands;
mod e2e;
pub(crate) mod e2e_vault;
pub(crate) mod gate;
pub(crate) mod grant_commands;
mod host;
mod hub;
pub(crate) mod interactive;
mod memory;
mod notes;
mod notify;
mod owner_notify;
mod recovery;
mod room_updates;
mod rule;
mod runner;
pub(crate) mod skill_commands;
mod store;
pub(crate) mod talk;
pub(crate) mod task_commands;
mod task_tick;
mod transcript;

#[cfg(test)]
mod account_tests;
#[cfg(test)]
mod e2e_tests;
#[cfg(test)]
mod memory_tests;
#[cfg(test)]
mod tests;

pub use account::AccountHost;
pub(crate) use gate::ApprovalGate;
pub use host::BotHost;
pub(crate) use owner_notify::notifier_for;

use account::AccountConfig;
use host::{HostConfig, TurnEvents, TurnInfo};
use std::sync::Arc;
use tauri::{AppHandle, Manager};

const TURN_EVENT: &str = "bot-host://turn";

struct UiEvents(AppHandle, notify::Seen);

impl TurnEvents for UiEvents {
    fn turn(&self, info: &TurnInfo) {
        crate::utils::emit_to_ui(&self.0, TURN_EVENT, info.clone());
    }

    fn alert(&self, alert: &notify::Alert) {
        if self.1.first(alert) {
            crate::utils::emit_to_ui(&self.0, notify::NOTIFICATION_EVENT, notify::batch(alert));
        }
    }
}

/// One id per installation, kept next to the app's data so the hub tells hosts apart.
fn host_id(app: &AppHandle) -> String {
    let path = app
        .path()
        .app_data_dir()
        .ok()
        .map(|dir| dir.join("bot-host-id"));
    let stored = path
        .as_ref()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .map(|id| id.trim().to_owned())
        .filter(|id| !id.is_empty() && !id.chars().any(char::is_whitespace));
    if let Some(id) = stored {
        return id;
    }
    let id = format!("desktop:{}", uuid::Uuid::new_v4().simple());
    if let Some(path) = path {
        let saved = path
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|()| std::fs::write(&path, &id));
        if let Err(error) = saved {
            tracing::warn!(%error, "Bot host: the host id could not be saved; it lasts until restart");
        }
    }
    id
}

/// Stops reporting the account bots this desktop runs (sign-out, exit); never waits long.
pub async fn withdraw(app: &AppHandle) {
    let Some(account) = app.try_state::<AccountHost>() else {
        return;
    };
    let _ = tokio::time::timeout(std::time::Duration::from_secs(3), account.withdraw()).await;
}

/// Manages the hosts and starts them; call once the local bots are managed. The local host runs
/// the bots of this device; the account host those of the signed-in account that live here.
pub fn start(app: &AppHandle) {
    let events = Arc::new(UiEvents(app.clone(), notify::Seen::default()));
    let runner = Arc::new(runner::NativeRunner(app.clone()));
    let (host, dispatcher) = BotHost::new(
        Arc::new(store::AppStore(app.clone())),
        runner.clone(),
        events.clone(),
        HostConfig::default(),
    );
    let hub = Arc::new(hub::ProfileHub::new(app.clone()));
    let account = AccountHost::new(
        hub.clone(),
        runner,
        events,
        host.slots(),
        AccountConfig::new(host_id(app)),
    );
    let e2e = Arc::new(e2e::E2eHost::new(
        hub,
        Arc::new(e2e_vault::KeychainVault::BOTS),
        host_id(app),
    ));
    account.enable_e2e(e2e.clone());
    app.manage(host);
    app.manage(account.clone());
    app.manage(e2e);
    tauri::async_runtime::spawn(dispatcher.run());
    tauri::async_runtime::spawn(account.run());
    tauri::async_runtime::spawn(expire_waits(app.clone()));
    tauri::async_runtime::spawn(task_tick::run_local(app.clone()));
}

/// Ends the local waits older than seven days, once an hour.
async fn expire_waits(app: AppHandle) {
    let mut tick = tokio::time::interval(std::time::Duration::from_secs(3_600));
    loop {
        tick.tick().await;
        let Some(bots) = app.try_state::<crate::functions::local_bots::LocalBots>() else {
            continue;
        };
        let expired =
            crate::functions::local_bots::on_store(&bots, |store| store.expire_parked()).await;
        if let Err(error) = expired {
            tracing::warn!(%error, "Bot host: ending abandoned approvals failed");
        }
    }
}

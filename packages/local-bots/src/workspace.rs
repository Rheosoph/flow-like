//! A bot's workspace: a local app of its own for its files and databases, hidden from every
//! app list and never in a profile, deleted with the bot. The store records which app it is
//! (`bots.workspace_app_id`, one bot per app); the host makes and deletes the apps and the
//! bots' memory through [`WorkspaceHost`]. Flux has none yet, as on the account.

use crate::{
    LocalBotStore,
    bots::{StoredBot, required_bot},
    error::{Error, Result, codes},
};
use flow_like_types::bot::BotSystemKind;
use rusqlite::params;
use std::{future::Future, sync::Arc};

/// The bot a workspace app is made for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkspaceBot<'a> {
    pub id: &'a str,
    /// The name shown, a built-in's default while it was never renamed.
    pub name: &'a str,
}

/// What a host does for workspaces. Each error says what went wrong, for the person to read.
/// The app a lost race made is deleted best effort: the host reports its own failure there.
pub trait WorkspaceHost: Sync {
    /// Makes the bot's hidden app, under `app_id` when given, and returns its id.
    fn create_app(
        &self,
        bot: WorkspaceBot<'_>,
        app_id: Option<&str>,
    ) -> impl Future<Output = Result<String, String>> + Send;

    /// Whether the app is on this device.
    fn app_exists(&self, app_id: &str) -> impl Future<Output = Result<bool, String>> + Send;

    /// Deletes the app with its files and databases.
    fn delete_app(&self, app_id: &str) -> impl Future<Output = Result<(), String>> + Send;

    /// Deletes everything the bot remembers; a bot without memory is fine.
    fn delete_memory(&self, bot_id: &str) -> impl Future<Output = Result<(), String>> + Send;
}

/// The bot's name and the workspace app it has, if any.
struct Slot {
    name: String,
    app_id: Option<String>,
}

fn refuse_flux(bot: &StoredBot) -> Result<()> {
    if bot.system == Some(BotSystemKind::Flux) {
        return Err(Error::new(
            codes::BOT_WORKSPACE_REFUSED,
            "Flux can't have a workspace yet",
        ));
    }
    Ok(())
}

fn host_failed(action: String) -> impl FnOnce(String) -> Error {
    move |reason| {
        Error::new(
            codes::BOT_WORKSPACE_FAILED,
            format!("Could not {action}: {reason}"),
        )
    }
}

/// Runs `work` on the store off the async runtime.
async fn blocking<T, F>(store: &Arc<LocalBotStore>, bot_id: &str, work: F) -> Result<T>
where
    T: Send + 'static,
    F: FnOnce(&LocalBotStore, &str) -> Result<T> + Send + 'static,
{
    let (store, bot_id) = (Arc::clone(store), bot_id.to_string());
    tokio::task::spawn_blocking(move || work(&store, &bot_id))
        .await
        .map_err(|error| Error::storage(format!("The local bots worker failed: {error}")))?
}

/// Makes the workspace's app again, empty and under its id, when it went missing.
async fn restore<H: WorkspaceHost>(host: &H, bot: WorkspaceBot<'_>, app_id: &str) -> Result<()> {
    let found = host.app_exists(app_id).await.map_err(host_failed(format!(
        "find app '{app_id}', the workspace of bot '{}'",
        bot.id
    )))?;
    if !found {
        host.create_app(bot, Some(app_id))
            .await
            .map_err(host_failed(format!(
                "make app '{app_id}', the workspace of bot '{}', again",
                bot.id
            )))?;
    }
    Ok(())
}

/// Deletes the workspace's app, when there is one, then the bot's memory.
async fn clear<H: WorkspaceHost>(host: &H, bot_id: &str, app_id: Option<&str>) -> Result<()> {
    if let Some(app_id) = app_id {
        let action = format!("delete app '{app_id}', the workspace of bot '{bot_id}'");
        let found = host
            .app_exists(app_id)
            .await
            .map_err(host_failed(action.clone()))?;
        if found {
            host.delete_app(app_id).await.map_err(host_failed(action))?;
        }
    }
    host.delete_memory(bot_id)
        .await
        .map_err(host_failed(format!("delete the memory of bot '{bot_id}'")))
}

impl LocalBotStore {
    fn workspace_slot(&self, bot_id: &str) -> Result<Slot> {
        self.read(|db| {
            let bot = required_bot(db, bot_id)?;
            refuse_flux(&bot)?;
            Ok(Slot {
                name: bot.display_name().to_string(),
                app_id: bot.workspace_app_id,
            })
        })
    }

    /// Records `app_id` as the bot's workspace unless it has one already; returns the one it
    /// has. The workspace isn't on the wire, so the bot's `updated_at` stays.
    fn claim_workspace(&self, bot_id: &str, app_id: &str) -> Result<String> {
        self.write(|tx, _| {
            let bot = required_bot(tx, bot_id)?;
            refuse_flux(&bot)?;
            if let Some(held) = bot.workspace_app_id {
                return Ok(held);
            }
            tx.execute(
                "UPDATE bots SET workspace_app_id = ?2 WHERE id = ?1",
                params![bot_id, app_id],
            )?;
            Ok(app_id.to_string())
        })
    }

    /// The bot's workspace app: made on first use, the same app on every call after. When
    /// first uses race, each makes an app, the first to record its own wins and every other
    /// deletes the app it made. A workspace whose app went missing comes back, empty, under
    /// its id. Refused for Flux.
    pub async fn workspace<H: WorkspaceHost>(
        self: &Arc<Self>,
        host: &H,
        bot_id: &str,
    ) -> Result<String> {
        let slot = blocking(self, bot_id, Self::workspace_slot).await?;
        let bot = WorkspaceBot {
            id: bot_id,
            name: &slot.name,
        };
        if let Some(app_id) = slot.app_id {
            restore(host, bot, &app_id).await?;
            return Ok(app_id);
        }
        let made = host
            .create_app(bot, None)
            .await
            .map_err(host_failed(format!("make a workspace for bot '{bot_id}'")))?;
        let claim = made.clone();
        let held = blocking(self, bot_id, move |store, bot_id| {
            store.claim_workspace(bot_id, &claim)
        })
        .await;
        if held.as_ref() != Ok(&made) {
            host.delete_app(&made).await.ok();
        }
        held
    }

    /// Deletes the bot with its workspace app and memory; Assistant and Flux stay. The app and
    /// the memory go first, so a failure there keeps the bot and deleting it again finishes the
    /// job; a workspace a first use made meanwhile goes once the bot is gone.
    pub async fn delete_bot_with_workspace<H: WorkspaceHost>(
        self: &Arc<Self>,
        host: &H,
        bot_id: &str,
    ) -> Result<()> {
        let held = blocking(self, bot_id, Self::deletable_bot).await?;
        clear(host, bot_id, held.as_deref()).await?;
        let held = blocking(self, bot_id, Self::delete_bot).await?;
        clear(host, bot_id, held.as_deref()).await
    }
}

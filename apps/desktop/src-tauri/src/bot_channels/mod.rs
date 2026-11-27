//! FlowBots in outside chat apps. Discord runs in this app only: while a user is signed in, the
//! gateway client of every enabled bot relays admitted messages to the hub and posts the
//! replies. The token never leaves the keychain.

mod discord;
mod hub;
mod store;

use discord::{LinkState, Running, Status};
use hub::Hub;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, PoisonError};
use std::time::Duration;
use store::{DiscordSettings, SettingsFile};
use tauri::{AppHandle, Manager, State};
use tokio::sync::{Mutex, Notify};

const SUPERVISE_EVERY: Duration = Duration::from_secs(15);
const SIGNED_OUT: &str = "Sign in to relay Discord messages";

#[derive(Default)]
struct Inner {
    running: HashMap<String, Running>,
    signed_in: bool,
}

pub struct BotChannels {
    app: AppHandle,
    file: SettingsFile,
    hub: Arc<Hub>,
    inner: Mutex<Inner>,
    wake: Notify,
}

#[derive(Debug, Serialize)]
pub struct BotDiscordView {
    enabled: bool,
    respond_to_mentions: bool,
    respond_to_dms: bool,
    channel_allow: Vec<String>,
    has_token: bool,
    status: Status,
    error: Option<String>,
    bot_user: Option<discord::BotUser>,
    invite_url: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct BotDiscordInput {
    enabled: bool,
    #[serde(default = "yes")]
    respond_to_mentions: bool,
    #[serde(default = "yes")]
    respond_to_dms: bool,
    #[serde(default)]
    channel_allow: Vec<String>,
    #[serde(default)]
    token: Option<String>,
}

fn yes() -> bool {
    true
}

fn failed(error: impl std::fmt::Display) -> String {
    error.to_string()
}

fn fingerprint(subject: &str, settings: &DiscordSettings, token: &str) -> String {
    let token = blake3::hash(token.as_bytes()).to_hex();
    format!(
        "{subject}|{}|{}|{}|{}",
        settings.respond_to_mentions,
        settings.respond_to_dms,
        settings.channel_allow.join(","),
        &token[..32]
    )
}

impl BotChannels {
    fn new(app: &AppHandle) -> Option<Self> {
        let dir = app.path().app_data_dir().ok()?;
        Some(Self {
            app: app.clone(),
            file: SettingsFile::new(&dir),
            hub: Arc::new(Hub::new(app.clone())),
            inner: Mutex::default(),
            wake: Notify::new(),
        })
    }

    async fn reconcile(&self, inner: &mut Inner) {
        let subject = hub::session(&self.app)
            .await
            .ok()
            .flatten()
            .map(|session| session.auth.subject.clone());
        inner.signed_in = subject.is_some();
        let all = self.file.load();
        inner.running.retain(|bot_id, running| {
            let keep = subject.is_some() && all.get(bot_id).is_some_and(|s| s.enabled);
            if !keep {
                running.stop();
            }
            keep
        });
        let Some(subject) = subject else { return };
        for (bot_id, settings) in all.iter().filter(|(_, settings)| settings.enabled) {
            let Ok(Some(token)) = store::token_of(bot_id) else {
                continue;
            };
            let fingerprint = fingerprint(&subject, settings, &token);
            let current = inner.running.get(bot_id);
            if current.is_some_and(|run| run.fingerprint == fingerprint && !run.needs_restart()) {
                continue;
            }
            if let Some(old) = current {
                old.stop();
            }
            let running = Running::start(self.hub.clone(), bot_id, settings, token, fingerprint);
            inner.running.insert(bot_id.clone(), running);
        }
    }

    async fn supervise(self: Arc<Self>) {
        loop {
            {
                let mut inner = self.inner.lock().await;
                self.reconcile(&mut inner).await;
            }
            tokio::select! {
                () = tokio::time::sleep(SUPERVISE_EVERY) => {}
                () = self.wake.notified() => {}
            }
        }
    }

    fn view(&self, inner: &Inner, bot_id: &str) -> Option<BotDiscordView> {
        let settings = self.file.load().remove(bot_id);
        let has_token = store::token_of(bot_id).ok().flatten().is_some();
        if settings.is_none() && !has_token {
            return None;
        }
        let settings = settings.unwrap_or(DiscordSettings {
            enabled: false,
            respond_to_mentions: true,
            respond_to_dms: true,
            channel_allow: Vec::new(),
        });
        let link = inner.running.get(bot_id).map(|running| {
            running
                .state
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone()
        });
        let link = match link {
            Some(link) => link,
            None => LinkState {
                status: Status::Stopped,
                error: (settings.enabled && !inner.signed_in).then(|| SIGNED_OUT.to_owned()),
                bot_user: None,
                invite_url: None,
            },
        };
        Some(BotDiscordView {
            enabled: settings.enabled,
            respond_to_mentions: settings.respond_to_mentions,
            respond_to_dms: settings.respond_to_dms,
            channel_allow: settings.channel_allow,
            has_token,
            status: link.status,
            error: link.error,
            bot_user: link.bot_user,
            invite_url: link.invite_url,
        })
    }

    async fn get(&self, bot_id: &str) -> Result<Option<BotDiscordView>, String> {
        store::check_bot_id(bot_id).map_err(failed)?;
        let inner = self.inner.lock().await;
        Ok(self.view(&inner, bot_id))
    }

    async fn set(&self, bot_id: &str, input: BotDiscordInput) -> Result<BotDiscordView, String> {
        store::check_bot_id(bot_id).map_err(failed)?;
        let settings = DiscordSettings {
            enabled: input.enabled,
            respond_to_mentions: input.respond_to_mentions,
            respond_to_dms: input.respond_to_dms,
            channel_allow: input.channel_allow,
        }
        .validated()
        .map_err(failed)?;
        let mut inner = self.inner.lock().await;
        match input.token.as_deref().map(str::trim) {
            Some("") => store::forget_token(bot_id).map_err(failed)?,
            Some(token) => store::keep_token(bot_id, token).map_err(failed)?,
            None => {}
        }
        let has_token = store::token_of(bot_id).map_err(failed)?.is_some();
        if settings.enabled && !has_token {
            return Err("Add the Discord bot token before turning Discord on".into());
        }
        let mut all = self.file.load();
        all.insert(bot_id.to_owned(), settings);
        self.file.save(&all).map_err(failed)?;
        self.reconcile(&mut inner).await;
        self.view(&inner, bot_id)
            .ok_or_else(|| "The Discord settings were not saved".into())
    }

    async fn remove(&self, bot_id: &str) -> Result<(), String> {
        store::check_bot_id(bot_id).map_err(failed)?;
        let mut inner = self.inner.lock().await;
        let mut all = self.file.load();
        if all.remove(bot_id).is_some() {
            self.file.save(&all).map_err(failed)?;
        }
        store::forget_token(bot_id).map_err(failed)?;
        self.reconcile(&mut inner).await;
        self.wake.notify_one();
        Ok(())
    }
}

pub fn start(app: &AppHandle) {
    let Some(channels) = BotChannels::new(app) else {
        tracing::warn!("Bot channels: no app data directory; Discord relay is off");
        return;
    };
    let channels = Arc::new(channels);
    app.manage(channels.clone());
    tauri::async_runtime::spawn(channels.supervise());
}

#[tauri::command]
pub async fn bot_discord_get(
    channels: State<'_, Arc<BotChannels>>,
    bot_id: String,
) -> Result<Option<BotDiscordView>, String> {
    channels.get(&bot_id).await
}

#[tauri::command]
pub async fn bot_discord_set(
    channels: State<'_, Arc<BotChannels>>,
    bot_id: String,
    input: BotDiscordInput,
) -> Result<BotDiscordView, String> {
    channels.set(&bot_id, input).await
}

#[tauri::command]
pub async fn bot_discord_remove(
    channels: State<'_, Arc<BotChannels>>,
    bot_id: String,
) -> Result<(), String> {
    channels.remove(&bot_id).await
}

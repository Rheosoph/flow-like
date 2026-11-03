//! The workspaces of this device's bots: each a local app of its own (Offline, marked with its
//! bot, never in a profile, hidden from every app list), made on first use and deleted with the
//! bot together with its memory, which lives in `<user_dir>/bot-memory/<bot id>`.

use super::{CommandError, LocalBots};
use crate::{
    functions::app,
    state::{TauriFlowLikeState, TauriSettingsState},
};
use flow_like::{
    app::App,
    bit::Metadata,
    flow_like_storage::{
        Path,
        object_store::{self, ObjectStoreExt},
    },
    state::FlowLikeState,
};
use flow_like_local_bots::{WorkspaceBot, WorkspaceHost};
use serde::Serialize;
use std::{
    path::{Path as FsPath, PathBuf},
    sync::Arc,
};
use tauri::{AppHandle, State};

/// The apps and the bots' memory of this desktop.
pub(crate) struct DesktopWorkspaces<'a> {
    app: &'a AppHandle,
}

impl<'a> DesktopWorkspaces<'a> {
    pub(crate) fn new(app: &'a AppHandle) -> Self {
        Self { app }
    }

    async fn state(&self) -> Result<Arc<FlowLikeState>, String> {
        TauriFlowLikeState::construct(self.app)
            .await
            .map_err(|error| error.to_string())
    }
}

impl WorkspaceHost for DesktopWorkspaces<'_> {
    async fn create_app(
        &self,
        bot: WorkspaceBot<'_>,
        app_id: Option<&str>,
    ) -> Result<String, String> {
        create_workspace_app(self.state().await?, bot, app_id)
            .await
            .map_err(|error| error.to_string())
    }

    async fn app_exists(&self, app_id: &str) -> Result<bool, String> {
        workspace_app_exists(&self.state().await?, app_id)
            .await
            .map_err(|error| error.to_string())
    }

    async fn delete_app(&self, app_id: &str) -> Result<(), String> {
        let deleted = app::delete_app(self.app.clone(), app_id.to_string())
            .await
            .map_err(|error| error.to_string());
        if let Err(error) = &deleted {
            tracing::warn!(app_id, %error, "Deleting a bot's workspace app failed");
        }
        deleted
    }

    async fn delete_memory(&self, bot_id: &str) -> Result<(), String> {
        let settings = TauriSettingsState::construct(self.app)
            .await
            .map_err(|error| error.to_string())?;
        let directory = memory_dir(&settings.lock().await.user_dir, bot_id);
        let shown = directory.display().to_string();
        tokio::task::spawn_blocking(move || remove_memory(&directory))
            .await
            .map_err(|error| error.to_string())?
            .map_err(|error| format!("removing {shown} failed: {error}"))
    }
}

fn workspace_metadata(bot: WorkspaceBot<'_>) -> Metadata {
    Metadata {
        name: format!("{}'s workspace", bot.name),
        description: format!("The files, databases and memory of bot {}", bot.name),
        ..Metadata::default()
    }
}

/// An app like `create_app` makes, marked as `bot`'s workspace and left out of every profile.
async fn create_workspace_app(
    state: Arc<FlowLikeState>,
    bot: WorkspaceBot<'_>,
    app_id: Option<&str>,
) -> flow_like_types::Result<String> {
    let mut app = App::new(
        app_id.map(str::to_string),
        workspace_metadata(bot),
        Vec::new(),
        state,
    )
    .await?;
    app.bot_id = Some(bot.id.to_string());
    app.save().await?;
    Ok(app.id)
}

async fn workspace_app_exists(
    state: &Arc<FlowLikeState>,
    app_id: &str,
) -> flow_like_types::Result<bool> {
    let store = FlowLikeState::project_meta_store(state).await?.as_generic();
    let manifest = Path::from("apps").join(app_id).join("manifest.app");
    match store.head(&manifest).await {
        Ok(_) => Ok(true),
        Err(object_store::Error::NotFound { .. }) => Ok(false),
        Err(error) => Err(flow_like_types::anyhow!(
            "Reading the manifest of app {app_id} failed: {error}"
        )),
    }
}

/// Where the desktop's `build_user_database` keeps `MemoryScope::Bot(bot_id)`.
fn memory_dir(user_dir: &FsPath, bot_id: &str) -> PathBuf {
    user_dir.join(Path::from("bot-memory").join(bot_id).to_string())
}

fn remove_memory(directory: &FsPath) -> std::io::Result<()> {
    match std::fs::remove_dir_all(directory) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        removed => removed,
    }
}

/// `{ app_id }`: the app that holds a bot's files and databases.
#[derive(Debug, Serialize)]
pub struct BotWorkspace {
    app_id: String,
}

/// The app of the local bot's workspace, made on first use.
pub(crate) async fn local_workspace(
    app: &AppHandle,
    bots: &LocalBots,
    bot_id: &str,
) -> Result<String, CommandError> {
    Ok(bots
        .store()?
        .workspace(&DesktopWorkspaces::new(app), bot_id)
        .await?)
}

/// The bot's workspace app, made on first use; a repeat returns the same app. Rejects with
/// `BOT_WORKSPACE_REFUSED` for Flux and `NOT_FOUND` for a bot that isn't on this device.
#[tauri::command]
pub async fn local_bots_workspace(
    app_handle: AppHandle,
    state: State<'_, LocalBots>,
    bot_id: String,
) -> Result<BotWorkspace, CommandError> {
    let app_id = local_workspace(&app_handle, &state, &bot_id).await?;
    Ok(BotWorkspace { app_id })
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like::{
        flow_like_storage::{files::store::FlowLikeStore, object_store::memory::InMemory},
        state::FlowLikeConfig,
        utils::http::HTTPClient,
    };

    fn state() -> Arc<FlowLikeState> {
        let store = FlowLikeStore::Other(Arc::new(InMemory::new()));
        let mut config = FlowLikeConfig::new();
        config.register_app_meta_store(store.clone());
        config.register_app_storage_store(store);
        Arc::new(FlowLikeState::new(
            config,
            HTTPClient::new_without_refetch(),
        ))
    }

    #[tokio::test]
    async fn a_workspace_is_an_offline_app_marked_with_its_bot_and_named_after_it() {
        let state = state();
        let bot = WorkspaceBot {
            id: "bot_scout",
            name: "Scout",
        };
        assert!(!workspace_app_exists(&state, "app-ws").await.unwrap());

        let made = create_workspace_app(state.clone(), bot, None)
            .await
            .unwrap();
        let again = create_workspace_app(state.clone(), bot, Some("app-ws"))
            .await
            .unwrap();

        assert_eq!(again, "app-ws");
        for app_id in [made, again] {
            assert!(workspace_app_exists(&state, &app_id).await.unwrap());
            let app = App::load(app_id.clone(), state.clone()).await.unwrap();
            assert_eq!(app.bot_id.as_deref(), Some("bot_scout"));
            assert!(matches!(
                app.visibility,
                flow_like::app::AppVisibility::Offline
            ));
            let meta = App::get_meta(app_id, state.clone(), None, None)
                .await
                .unwrap();
            assert_eq!(meta.name, "Scout's workspace");
        }
    }

    #[test]
    fn a_bots_memory_is_removed_where_the_desktop_keeps_it() {
        let user_dir = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        let memory = memory_dir(&user_dir, "bot_scout");
        let other = memory_dir(&user_dir, "bot_keeper");
        assert_eq!(memory, user_dir.join("bot-memory").join("bot_scout"));
        assert_eq!(
            memory_dir(&user_dir, "../bot_keeper").parent(),
            Some(user_dir.join("bot-memory").as_path()),
            "a bot id stays one path segment"
        );
        for directory in [&memory, &other] {
            std::fs::create_dir_all(directory.join("memory.lance")).unwrap();
            std::fs::write(directory.join("memory.lance").join("data"), b"facts").unwrap();
        }

        remove_memory(&memory).unwrap();
        remove_memory(&memory).unwrap();

        assert!(!memory.exists());
        assert!(other.join("memory.lance").join("data").is_file());
        std::fs::remove_dir_all(user_dir).unwrap();
    }
}

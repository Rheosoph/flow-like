//! The skills of local bots, for the bot settings. A zip is read in this process; its
//! references land in the bot's workspace under `bot/skills/<name>/`.

use crate::{
    functions::local_bots::{CommandError, LocalBots, on_store, workspace::local_workspace},
    state::TauriFlowLikeState,
};
use flow_like::flow_like_storage::object_store::ObjectStoreExt;
use flow_like_bot_tools::skills::import::unpack;
use flow_like_local_bots::SkillOrigin;
use flow_like_types::bot_skill::{
    BotSkill, CreateBotSkill, SkillStatus, UpdateBotSkill, skill_dir,
};
use futures::TryStreamExt;
use tauri::{AppHandle, State};

/// The skills of a local bot with their status, by name.
#[tauri::command]
pub async fn local_bot_skills(
    bots: State<'_, LocalBots>,
    bot_id: String,
) -> Result<Vec<BotSkill>, String> {
    on_store(&bots, move |store| store.skills(&bot_id))
        .await
        .map_err(|error| error.to_string())
}

/// A skill the person wrote: active at once.
#[tauri::command]
pub async fn local_bot_create_skill(
    bots: State<'_, LocalBots>,
    bot_id: String,
    request: CreateBotSkill,
) -> Result<BotSkill, String> {
    on_store(&bots, move |store| {
        store.create_skill(&bot_id, request, SkillOrigin::owner())
    })
    .await
    .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn local_bot_update_skill(
    bots: State<'_, LocalBots>,
    bot_id: String,
    skill_id: String,
    request: UpdateBotSkill,
) -> Result<BotSkill, String> {
    on_store(&bots, move |store| {
        store.update_skill(&bot_id, &skill_id, request)
    })
    .await
    .map_err(|error| error.to_string())
}

/// Approves (`active`) or switches off (`disabled`) a skill.
#[tauri::command]
pub async fn local_bot_set_skill_status(
    bots: State<'_, LocalBots>,
    bot_id: String,
    skill_id: String,
    status: SkillStatus,
) -> Result<BotSkill, String> {
    on_store(&bots, move |store| {
        store.set_skill_status(&bot_id, &skill_id, status)
    })
    .await
    .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn local_bot_delete_skill(
    app: AppHandle,
    bots: State<'_, LocalBots>,
    bot_id: String,
    skill_id: String,
) -> Result<(), String> {
    let lookup = bot_id.clone();
    let removed = on_store(&bots, move |store| store.delete_skill(&lookup, &skill_id))
        .await
        .map_err(|error| error.to_string())?;
    if let Err(error) = remove_references(&app, &bots, &bot_id, &removed.name).await {
        tracing::warn!(%error, bot_id, name = %removed.name, "A skill's files were not removed");
    }
    Ok(())
}

/// Reads a zipped skill folder: the skill is saved as pending and its files go to the bot's
/// workspace. A zip that breaks the limits writes nothing.
#[tauri::command]
pub async fn local_bot_import_skill(
    app: AppHandle,
    bots: State<'_, LocalBots>,
    bot_id: String,
    zip: Vec<u8>,
) -> Result<BotSkill, String> {
    let unpacked = tokio::task::spawn_blocking(move || unpack(&zip))
        .await
        .map_err(|error| format!("The skill reader stopped: {error}"))??;
    let workspace = local_workspace(&app, &bots, &bot_id)
        .await
        .map_err(|error| error.to_string())?;
    let store = workspace_store(&app).await?;
    let folder = skill_dir(&unpacked.skill.name);
    for (relative, content) in unpacked.files {
        let location = store
            .construct_upload(&workspace, &format!("{folder}/{relative}"))
            .await
            .map_err(|error| error.to_string())?;
        store
            .as_generic()
            .put(&location, content.into())
            .await
            .map_err(|error| error.to_string())?;
    }
    let request = CreateBotSkill {
        name: unpacked.skill.name,
        description: unpacked.skill.description,
        body: unpacked.skill.body,
        tools: Vec::new(),
    };
    on_store(&bots, move |store| store.reimport_skill(&bot_id, request))
        .await
        .map_err(|error| error.to_string())
}

async fn workspace_store(
    app: &AppHandle,
) -> Result<flow_like::flow_like_storage::files::store::FlowLikeStore, String> {
    let state = TauriFlowLikeState::construct(app)
        .await
        .map_err(|error| error.to_string())?;
    let store = state.config.read().await.stores.app_storage_store.clone();
    store.ok_or_else(|| "This device has no app storage".to_string())
}

async fn remove_references(
    app: &AppHandle,
    bots: &LocalBots,
    bot_id: &str,
    name: &str,
) -> Result<(), CommandError> {
    let workspace = local_workspace(app, bots, bot_id).await?;
    let store = workspace_store(app)
        .await
        .map_err(|error| CommandError::new("STORAGE", error))?;
    let fail = |error: String| CommandError::new("STORAGE", error);
    let prefix = store
        .construct_upload(&workspace, &skill_dir(name))
        .await
        .map_err(|error| fail(error.to_string()))?;
    let generic = store.as_generic();
    let found: Vec<_> = generic
        .list(Some(&prefix))
        .try_collect()
        .await
        .map_err(|error| fail(error.to_string()))?;
    for meta in found {
        let _ = generic.delete(&meta.location).await;
    }
    Ok(())
}

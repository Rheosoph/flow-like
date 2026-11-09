//! The skills of local bots, for the bot settings. A zip is read in this process; its
//! references land in the bot's workspace under `bot/skills/<name>/`.

use crate::{
    functions::local_bots::{LocalBots, on_store, workspace::local_workspace},
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
    if let Some(package_id) = removed.source.package_id().map(str::to_string) {
        release_package(&app, &bots, &package_id).await;
    }
    Ok(())
}

/// Uninstalls a package this device installed only for skills once no skill uses it.
async fn release_package(app: &AppHandle, bots: &LocalBots, package_id: &str) {
    let lookup = package_id.to_string();
    let release = on_store(bots, move |store| store.release_package(&lookup)).await;
    if matches!(release, Ok(true))
        && let Err(error) = crate::functions::registry::registry_uninstall_package(
            app.clone(),
            package_id.to_string(),
        )
        .await
    {
        tracing::warn!(%error, package_id, "The package of a deleted skill was not uninstalled");
    }
}

/// Installs the skills of a registry package for a local bot, as pending. The package is
/// installed on this device first when it is not (and removed again with its last skill); a
/// package that needs capabilities the bot lacks is refused.
#[tauri::command]
pub async fn local_bot_install_skill_package(
    app: AppHandle,
    bots: State<'_, LocalBots>,
    bot_id: String,
    package_id: String,
    version: Option<String>,
) -> Result<Vec<BotSkill>, String> {
    let client = crate::state::TauriRegistryState::get_client(&app)
        .await
        .map_err(|error| error.to_string())?;
    let mut installed_here = false;
    let mut package = client.get_installed(&package_id).await;
    if package.is_none()
        || version
            .as_ref()
            .is_some_and(|v| package.as_ref().is_some_and(|p| &p.version != v))
    {
        crate::functions::registry::registry_install_package(
            app.clone(),
            package_id.clone(),
            version.clone(),
            None,
            None,
        )
        .await
        .map_err(|error| error.to_string())?;
        installed_here = package.is_none();
        package = client.get_installed(&package_id).await;
    }
    let package =
        package.ok_or_else(|| format!("Package '{package_id}' could not be installed"))?;
    let entries: Vec<CreateBotSkill> = package
        .manifest
        .skills
        .iter()
        .map(|entry| {
            entry.validate()?;
            Ok(CreateBotSkill {
                name: entry.name.clone(),
                description: entry.description.clone(),
                body: entry.instructions.clone().unwrap_or_default(),
                tools: entry.tools.clone(),
            })
        })
        .collect::<Result<_, String>>()?;
    let tags = package.manifest.permissions.capability_tags();
    let version = package.manifest.version.clone();
    let lookup = package_id.clone();
    let saved = on_store(&bots, move |store| {
        store.install_package_skills(&bot_id, &lookup, &version, entries, &tags, installed_here)
    })
    .await
    .map_err(|error| error.to_string());
    if saved.is_err() {
        release_package(&app, &bots, &package_id).await;
    }
    saved
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
) -> Result<(), String> {
    let workspace = local_workspace(app, bots, bot_id)
        .await
        .map_err(|error| error.to_string())?;
    let store = workspace_store(app).await?;
    let fail = |error: String| error;
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

use crate::functions::local_bots::{LocalBots, on_store};
use flow_like_types::bot_task::{BotTask, CreateBotTask, TaskCreator, UpdateBotTask};
use tauri::State;

/// The scheduled tasks of a local bot, soonest first.
#[tauri::command]
pub async fn local_bot_tasks(
    bots: State<'_, LocalBots>,
    bot_id: String,
) -> Result<Vec<BotTask>, String> {
    on_store(&bots, move |store| store.tasks(&bot_id))
        .await
        .map_err(|error| error.to_string())
}

/// Schedules a task the person made for a local bot with the scheduling capability.
#[tauri::command]
pub async fn local_bot_create_task(
    bots: State<'_, LocalBots>,
    bot_id: String,
    request: CreateBotTask,
) -> Result<BotTask, String> {
    on_store(&bots, move |store| {
        store.create_task(&bot_id, request, TaskCreator::Owner)
    })
    .await
    .map_err(|error| error.to_string())
}

/// Pauses, resumes or edits a task; moving its schedule computes the next run from now.
#[tauri::command]
pub async fn local_bot_update_task(
    bots: State<'_, LocalBots>,
    bot_id: String,
    task_id: String,
    request: UpdateBotTask,
) -> Result<BotTask, String> {
    on_store(&bots, move |store| {
        store.update_task(&bot_id, &task_id, request)
    })
    .await
    .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn local_bot_delete_task(
    bots: State<'_, LocalBots>,
    bot_id: String,
    task_id: String,
) -> Result<(), String> {
    on_store(&bots, move |store| store.delete_task(&bot_id, &task_id))
        .await
        .map_err(|error| error.to_string())
}

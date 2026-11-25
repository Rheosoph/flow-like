//! The memory tab's calls for a bot's facts, on the bot's workspace database: list them with the
//! chat positions they came from, forget one the way `memory_forget` does, reset the memory.

use crate::state::TauriFlowLikeState;
use flow_like::flow::copilot::memory_open::{forget_bot_memory, workspace_connection};
use flow_like_bot_memory::host::{EntityUpdate, EntityView, FactAdmin, FactView};
use tauri::State;

async fn admin(state: &TauriFlowLikeState, app_id: &str) -> Result<FactAdmin, String> {
    let (connection, write_options) = workspace_connection(&state.0, app_id, None)
        .await
        .map_err(|error| error.to_string())?;
    Ok(FactAdmin::open(connection, write_options).await)
}

#[tauri::command]
pub async fn bot_memory_facts(
    state: State<'_, TauriFlowLikeState>,
    app_id: String,
    include_superseded: Option<bool>,
) -> Result<Vec<FactView>, String> {
    let facts = admin(&state, &app_id)
        .await?
        .list_views(include_superseded.unwrap_or(false))
        .await
        .map_err(|error| error.to_string())?;
    Ok(facts)
}

#[tauri::command]
pub async fn bot_memory_forget(
    state: State<'_, TauriFlowLikeState>,
    app_id: String,
    fact_id: String,
) -> Result<FactView, String> {
    admin(&state, &app_id)
        .await?
        .forget(&fact_id)
        .await
        .map(FactView::from)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn bot_memory_reset(
    state: State<'_, TauriFlowLikeState>,
    app_id: String,
    bot_id: String,
) -> Result<(), String> {
    admin(&state, &app_id)
        .await?
        .reset()
        .await
        .map_err(|error| error.to_string())?;
    forget_bot_memory(&bot_id);
    Ok(())
}

#[tauri::command]
pub async fn bot_memory_entities(
    state: State<'_, TauriFlowLikeState>,
    app_id: String,
) -> Result<Vec<EntityView>, String> {
    admin(&state, &app_id)
        .await?
        .list_entities()
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn bot_memory_entity_update(
    state: State<'_, TauriFlowLikeState>,
    app_id: String,
    entity_id: String,
    scope: Option<String>,
    name: Option<String>,
    kind: Option<String>,
) -> Result<EntityView, String> {
    admin(&state, &app_id)
        .await?
        .update_entity(scope.as_deref(), &entity_id, &EntityUpdate { name, kind })
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn bot_memory_entity_merge(
    state: State<'_, TauriFlowLikeState>,
    app_id: String,
    entity_id: String,
    scope: Option<String>,
    into: String,
) -> Result<EntityView, String> {
    admin(&state, &app_id)
        .await?
        .merge_entities(scope.as_deref(), &entity_id, &into)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn bot_memory_entity_forget(
    state: State<'_, TauriFlowLikeState>,
    app_id: String,
    entity_id: String,
    scope: Option<String>,
) -> Result<Vec<FactView>, String> {
    admin(&state, &app_id)
        .await?
        .forget_entity(scope.as_deref(), &entity_id)
        .await
        .map_err(|error| error.to_string())
}

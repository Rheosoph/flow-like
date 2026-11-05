use super::{approvals, store::{AppStore, HostStore}};
use crate::functions::local_bots::{LocalBots, on_store};
use flow_like_types::tool_grant::{ApprovalDecision, ToolGrant, ToolGrantKey};
use std::sync::Arc;
use tauri::{AppHandle, Manager, State};

/// What the person allowed a local bot to run without asking again.
#[tauri::command]
pub async fn local_bot_tool_grants(
    bots: State<'_, LocalBots>,
    bot_id: String,
) -> Result<Vec<ToolGrant>, String> {
    on_store(&bots, move |store| store.tool_grants(&bot_id))
        .await
        .map_err(|error| error.to_string())
}

/// Allows `tool` for `scope`; allowing again changes nothing.
#[tauri::command]
pub async fn local_bot_put_tool_grant(
    bots: State<'_, LocalBots>,
    bot_id: String,
    tool: String,
    scope: String,
) -> Result<ToolGrant, String> {
    on_store(&bots, move |store| {
        store.grant_tool(&bot_id, &ToolGrantKey { tool, scope })
    })
    .await
    .map_err(|error| error.to_string())
}

/// Takes a grant back; false when the bot had none.
#[tauri::command]
pub async fn local_bot_delete_tool_grant(
    bots: State<'_, LocalBots>,
    bot_id: String,
    tool: String,
    scope: String,
) -> Result<bool, String> {
    on_store(&bots, move |store| {
        store.revoke_tool(&bot_id, &ToolGrantKey { tool, scope })
    })
    .await
    .map_err(|error| error.to_string())
}

/// Answers the approval card a parked local turn waits on. `decision` is `once`, `always` or
/// `deny`; the turn runs again afterwards.
#[tauri::command]
pub async fn local_bot_answer_approval(
    app: AppHandle,
    bot_id: String,
    turn_id: String,
    approval_id: String,
    decision: ApprovalDecision,
) -> Result<(), String> {
    let store: Arc<dyn HostStore> = Arc::new(AppStore(app.clone()));
    approvals::answer_local(&store, &bot_id, &turn_id, &approval_id, decision).await?;
    if let Some(host) = app.try_state::<super::BotHost>() {
        host.poke(None);
    }
    Ok(())
}

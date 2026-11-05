//! Moving bots of this device into the signed-in account, for the webview and any other caller:
//! `local_bots_check_move` reviews a bot, `local_bots_move` moves it and tells every step as
//! `local-bots://move` (`{ bot_id, step, pass }`, the steps in `MoveStep`: `workspace`,
//! `segments` and `forget_workspace` only when the bot has them). Moves run one at a time.

use super::{CommandError, LocalBots, hub::AccountSession};
use flow_like_local_bots::{
    LocalBotStore, MoveError, MoveProgress, MoveReport, MoveStep, MovedBot,
};
use std::sync::Arc;
use tauri::{AppHandle, State};
use zeroize::Zeroizing;

const MOVE_EVENT: &str = "local-bots://move";

fn opened_store(state: &LocalBots) -> Result<Arc<LocalBotStore>, MoveError> {
    state
        .store()
        .map_err(|error: CommandError| MoveError::new(MoveStep::Check, error.code, error.message))
}

/// What moving the bot takes along and what blocks it; changes nothing. Rejects with
/// `{ code, message, step }`.
#[tauri::command]
pub async fn local_bots_check_move(
    app_handle: AppHandle,
    state: State<'_, LocalBots>,
    bot_id: String,
    access_token: String,
) -> Result<MoveReport, MoveError> {
    let access_token = Zeroizing::new(access_token);
    let store = opened_store(&state)?;
    let account = AccountSession::open(&app_handle, &access_token, &bot_id).await?;
    state.mover.check(&store, &account, &bot_id).await
}

/// Moves the bot with its apps, secrets, workspace and chats (with its summaries of them) into
/// the account, which gives them their ids, then removes it here:
/// `{ bot, conversations: [{ local_id, account_id }], warnings }`. Rejects with
/// `{ code, message, step, issue?, conversations? }`; repeating a move resumes it.
#[tauri::command]
pub async fn local_bots_move(
    app_handle: AppHandle,
    state: State<'_, LocalBots>,
    bot_id: String,
    access_token: String,
) -> Result<MovedBot, MoveError> {
    let access_token = Zeroizing::new(access_token);
    let store = opened_store(&state)?;
    let account = AccountSession::open(&app_handle, &access_token, &bot_id).await?;
    let progress =
        |progress: MoveProgress| crate::utils::emit_to_ui(&app_handle, MOVE_EVENT, progress);
    let moved = state
        .mover
        .move_bot(&store, &account, &bot_id, &progress)
        .await;
    if let Err(error) = &moved {
        tracing::warn!(bot_id = %bot_id, code = %error.code, step = ?error.step, "Moving a local bot to the account stopped");
    }
    moved
}

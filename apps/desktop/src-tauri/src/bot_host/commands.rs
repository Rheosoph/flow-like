use super::{AccountHost, BotHost, e2e::{E2eHost, E2eStatus}, host::TurnInfo};
use tauri::State;

/// Running and queued turns of local bots and of the account's bots that run here, then the
/// latest finished ones.
#[tauri::command]
pub fn bot_host_turns(host: State<'_, BotHost>, account: State<'_, AccountHost>) -> Vec<TurnInfo> {
    let mut turns = host.turns();
    turns.extend(account.turns());
    turns
}

/// Cancels a turn without a reply; false when it already finished or is posting its reply. An
/// account turn is cancelled at the hub (`POST /bots/{bot}/turns/{turn}/cancel`).
#[tauri::command]
pub async fn bot_host_cancel_turn(
    host: State<'_, BotHost>,
    account: State<'_, AccountHost>,
    turn_id: String,
) -> Result<bool, String> {
    Ok(host.cancel(&turn_id) || account.cancel(&turn_id).await)
}

/// The encryption state of an account bot on this desktop: key id, fingerprint, whether the
/// key is published and the host that holds it.
#[tauri::command]
pub fn bot_host_e2e_status(e2e: State<'_, std::sync::Arc<E2eHost>>, bot_id: String) -> E2eStatus {
    e2e.status(&bot_id)
}

/// Publishes a new inbox key for the bot and revokes the old one; earlier messages still open.
#[tauri::command]
pub async fn bot_host_e2e_rotate_key(
    e2e: State<'_, std::sync::Arc<E2eHost>>,
    bot_id: String,
) -> Result<E2eStatus, String> {
    e2e.rotate(&bot_id).await
}

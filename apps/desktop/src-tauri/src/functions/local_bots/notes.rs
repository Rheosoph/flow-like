//! The owner's view of the notes a local bot keeps in a conversation: read them, edit the
//! body or status, close or reopen. A stale `expected_version` rejects with a message that
//! starts with `conflict:` so the editor can reload instead of overwriting.

use super::{CommandError, LocalBots, on_store};
use flow_like_local_bots::{LOCAL_OWNER_ID, codes};
use flow_like_types::bot_notes::{BotNote, NoteAuthor, NoteWriteRequest};
use flow_like_types::data_classification::DataSensitivity;
use tauri::State;

const CONFLICT_PREFIX: &str = "conflict: ";

fn tell_conflict(error: CommandError) -> CommandError {
    if error.code == codes::NOTE_CONFLICT && !error.message.starts_with(CONFLICT_PREFIX) {
        return CommandError::new(error.code, format!("{CONFLICT_PREFIX}{}", error.message));
    }
    error
}

#[tauri::command]
pub async fn local_bots_notes_list(
    state: State<'_, LocalBots>,
    bot_id: String,
    conversation_id: String,
) -> Result<Vec<BotNote>, CommandError> {
    on_store(&state, move |store| {
        store.list_notes(&bot_id, &conversation_id)
    })
    .await
}

#[tauri::command]
pub async fn local_bots_notes_get(
    state: State<'_, LocalBots>,
    bot_id: String,
    conversation_id: String,
    key: String,
) -> Result<BotNote, CommandError> {
    on_store(&state, move |store| {
        store.get_note(&bot_id, &conversation_id, &key)
    })
    .await
}

#[tauri::command]
pub async fn local_bots_notes_update(
    state: State<'_, LocalBots>,
    bot_id: String,
    conversation_id: String,
    key: String,
    write: NoteWriteRequest,
) -> Result<BotNote, CommandError> {
    let author = NoteAuthor {
        by: LOCAL_OWNER_ID.to_owned(),
        at: chrono::Utc::now().timestamp_millis(),
        sensitivity: DataSensitivity::default(),
    };
    on_store(&state, move |store| {
        store.get_note(&bot_id, &conversation_id, &key)?;
        store.write_note(
            &bot_id,
            &conversation_id,
            &key,
            &write.change,
            &author,
            write.expected_version,
        )
    })
    .await
    .map_err(tell_conflict)
}

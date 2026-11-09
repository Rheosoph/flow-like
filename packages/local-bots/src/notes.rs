//! Notes a bot keeps in one conversation (see `flow_like_types::bot_notes`). Every write goes
//! through [`BotNote::apply`], so versions, history and sensitivity follow the API's rule.

use crate::{
    LocalBotStore,
    error::{Error, Result, codes},
};
use flow_like_types::bot_notes::{
    BotNote, NoteAuthor, NoteChange, NoteState, NoteVersion, prunable,
};
use flow_like_types::data_classification::{DataClassification, DataSensitivity};
use rusqlite::{Connection, OptionalExtension, Row, params};

const COLUMNS: &str = "key, title, status, body, state, version, classification, pii, \
    updated_by, history, created_at, updated_at";

fn note_of(row: &Row<'_>) -> rusqlite::Result<(BotNote, String)> {
    let state: String = row.get(4)?;
    let classification: Option<String> = row.get(6)?;
    let history: String = row.get(9)?;
    let note = BotNote {
        key: row.get(0)?,
        title: row.get(1)?,
        status: row.get(2)?,
        body: row.get(3)?,
        state: NoteState::from_stored(&state),
        version: row.get(5)?,
        sensitivity: DataSensitivity {
            classification: classification.as_deref().and_then(DataClassification::parse),
            pii: row.get(7)?,
        },
        updated_by: row.get(8)?,
        created_at: row.get(10)?,
        updated_at: row.get(11)?,
        history: Vec::new(),
    };
    Ok((note, history))
}

fn with_history(parsed: (BotNote, String)) -> Result<BotNote> {
    let (mut note, history) = parsed;
    note.history = serde_json::from_str::<Vec<NoteVersion>>(&history)?;
    Ok(note)
}

fn fetch(
    db: &Connection,
    bot_id: &str,
    conversation_id: &str,
    key: &str,
) -> Result<Option<BotNote>> {
    db.query_row(
        &format!(
            "SELECT {COLUMNS} FROM bot_notes WHERE bot_id = ?1 AND conversation_id = ?2 AND key = ?3"
        ),
        params![bot_id, conversation_id, key],
        note_of,
    )
    .optional()?
    .map(with_history)
    .transpose()
}

fn list(db: &Connection, bot_id: &str, conversation_id: &str) -> Result<Vec<BotNote>> {
    let mut rows = db.prepare_cached(&format!(
        "SELECT {COLUMNS} FROM bot_notes WHERE bot_id = ?1 AND conversation_id = ?2
         ORDER BY updated_at DESC, key"
    ))?;
    let notes = rows
        .query_map(params![bot_id, conversation_id], note_of)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(notes
        .into_iter()
        .map(|(note, _)| note)
        .collect())
}

impl LocalBotStore {
    /// The bot's notes in a conversation, newest first, without their history.
    pub fn list_notes(&self, bot_id: &str, conversation_id: &str) -> Result<Vec<BotNote>> {
        self.read(|db| list(db, bot_id, conversation_id))
    }

    /// One note with its kept versions.
    pub fn get_note(&self, bot_id: &str, conversation_id: &str, key: &str) -> Result<BotNote> {
        self.read(|db| fetch(db, bot_id, conversation_id, key))?
            .ok_or_else(|| Error::not_found(format!("This bot keeps no note '{key}' here")))
    }

    /// Creates or revises a note in one transaction and prunes the oldest closed notes beyond
    /// the keep limit. A stale `expected_version` is refused with [`codes::NOTE_CONFLICT`].
    pub fn write_note(
        &self,
        bot_id: &str,
        conversation_id: &str,
        key: &str,
        change: &NoteChange,
        author: &NoteAuthor,
        expected_version: Option<u32>,
    ) -> Result<BotNote> {
        self.write(|tx, _changes| {
            let existing = fetch(tx, bot_id, conversation_id, key)?;
            if let Some(expected) = expected_version
                && existing.as_ref().map(|note| note.version) != Some(expected)
            {
                return Err(Error::new(
                    codes::NOTE_CONFLICT,
                    format!("Note '{key}' changed since version {expected}"),
                ));
            }
            let note = BotNote::apply(existing.as_ref(), key, change, author)
                .map_err(|error| Error::invalid(error.to_string()))?;
            if existing.as_ref() == Some(&note) {
                return Ok(note);
            }
            tx.execute(
                "INSERT INTO bot_notes (bot_id, conversation_id, key, title, status, body, state,
                    version, classification, pii, updated_by, history, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
                 ON CONFLICT (bot_id, conversation_id, key) DO UPDATE SET
                    title = excluded.title, status = excluded.status, body = excluded.body,
                    state = excluded.state, version = excluded.version,
                    classification = excluded.classification, pii = excluded.pii,
                    updated_by = excluded.updated_by, history = excluded.history,
                    updated_at = excluded.updated_at",
                params![
                    bot_id,
                    conversation_id,
                    note.key,
                    note.title,
                    note.status,
                    note.body,
                    note.state.stored(),
                    note.version,
                    note.sensitivity.classification.map(|level| level.as_str()),
                    note.sensitivity.pii,
                    note.updated_by,
                    serde_json::to_string(&note.history)?,
                    note.created_at,
                    note.updated_at,
                ],
            )?;
            for pruned in prunable(&list(tx, bot_id, conversation_id)?) {
                tx.execute(
                    "DELETE FROM bot_notes WHERE bot_id = ?1 AND conversation_id = ?2 AND key = ?3",
                    params![bot_id, conversation_id, pruned],
                )?;
            }
            Ok(note)
        })
    }
}

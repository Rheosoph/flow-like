//! Conversation files on this device: a file the person (or a local bot) puts into a conversation
//! is kept in the `conversation_files` directory next to the store's file, one subdirectory per
//! conversation, and a row says who uploaded it. A message carries it as an attachment whose handle
//! names the conversation, exactly as the API's files do. Deleting the message, or the
//! conversation, removes the bytes.

use super::active_membership;
use crate::{
    error::{Error, Result, codes},
    store::{Changes, LocalBotStore, now_micros},
};
use flow_like_types::{
    bot_message::{
        CONVERSATION_FILE_APP_PREFIX, MAX_CONVERSATION_FILE_BYTES, MAX_FILE_NAME_CHARS,
        WorkspaceHandle, is_mime,
    },
    create_id, mime_guess,
};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};

pub(crate) const FILES_DIR: &str = "conversation_files";
const UNSENT_GRACE_MICROS: i64 = 24 * 60 * 60 * 1_000_000;

/// A stored file: put `app_id`, `path`, `bytes`, `sha256` and `mime` into the attachment part's
/// handle and `name` into its `name`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalConversationFile {
    pub file_id: String,
    pub name: String,
    pub app_id: String,
    pub path: String,
    pub bytes: u64,
    pub sha256: String,
    pub mime: String,
}

/// A conversation file a message points at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct FileUse {
    pub conversation_id: String,
    pub file_id: String,
    pub handle: WorkspaceHandle,
}

struct Row {
    id: String,
    uploader_id: String,
    name: String,
    mime: String,
    bytes: i64,
    sha256: String,
    message_id: Option<String>,
}

impl Row {
    fn ready(&self, conversation_id: &str) -> LocalConversationFile {
        LocalConversationFile {
            file_id: self.id.clone(),
            name: self.name.clone(),
            app_id: format!("{CONVERSATION_FILE_APP_PREFIX}{conversation_id}"),
            path: stored_name(&self.id, &self.name),
            bytes: self.bytes.max(0) as u64,
            sha256: self.sha256.clone(),
            mime: self.mime.clone(),
        }
    }
}

fn stored_name(id: &str, name: &str) -> String {
    let extension = name
        .rsplit_once('.')
        .map(|(_, extension)| extension.trim().to_ascii_lowercase())
        .filter(|extension| {
            (1..=16).contains(&extension.len())
                && extension.bytes().all(|byte| byte.is_ascii_alphanumeric())
        });
    match extension {
        Some(extension) => format!("{id}.{extension}"),
        None => id.to_string(),
    }
}

fn file_row(db: &Connection, conversation_id: &str, file_id: &str) -> Result<Option<Row>> {
    Ok(db
        .prepare_cached(
            "SELECT id, uploader_id, name, mime, bytes, sha256, message_id
             FROM conversation_files WHERE id = ?1 AND conversation_id = ?2",
        )?
        .query_row(params![file_id, conversation_id], |row| {
            Ok(Row {
                id: row.get(0)?,
                uploader_id: row.get(1)?,
                name: row.get(2)?,
                mime: row.get(3)?,
                bytes: row.get(4)?,
                sha256: row.get(5)?,
                message_id: row.get(6)?,
            })
        })
        .optional()?)
}

fn safe_component(value: &str, what: &str) -> Result<()> {
    if flow_like_types::inbox::is_valid_message_id(value) {
        return Ok(());
    }
    Err(Error::invalid(format!(
        "{what} '{}' must be 1 to 64 characters of A-Z, a-z, 0-9, '_' and '-'",
        value.chars().take(80).collect::<String>()
    )))
}

fn validate_upload(name: &str, mime: Option<&str>, data: &[u8]) -> Result<(String, String)> {
    let name = name.trim();
    let length = name.chars().count();
    if !(1..=MAX_FILE_NAME_CHARS).contains(&length) || name.chars().any(char::is_control) {
        return Err(Error::invalid(format!(
            "Upload name must be 1 to {MAX_FILE_NAME_CHARS} characters without line breaks; got {length} characters"
        )));
    }
    if data.is_empty() {
        return Err(Error::invalid(format!(
            "File '{name}' is empty; a conversation file has at least 1 byte"
        )));
    }
    if data.len() as u64 > MAX_CONVERSATION_FILE_BYTES {
        return Err(Error::invalid(format!(
            "File '{name}' is {} bytes; a conversation file has at most {MAX_CONVERSATION_FILE_BYTES} bytes",
            data.len()
        )));
    }
    let mime = mime
        .map(|mime| {
            mime.split(';')
                .next()
                .unwrap_or_default()
                .trim()
                .to_ascii_lowercase()
        })
        .filter(|mime| is_mime(mime))
        .or_else(|| {
            mime_guess::from_path(name)
                .first()
                .map(|mime| mime.to_string())
        })
        .unwrap_or_else(|| "application/octet-stream".to_string());
    Ok((name.to_string(), mime))
}

fn sha256_hex(data: &[u8]) -> String {
    Sha256::digest(data)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Ties the files a message carries to it, with the API's rules: ready files of this conversation
/// that the sender uploaded, with exactly the size, hash and type the handle states, not carried
/// by another message already.
pub(super) fn claim(
    db: &Connection,
    conversation_id: &str,
    sender_id: &str,
    message_id: &str,
    uses: &[FileUse],
) -> Result<()> {
    for used in uses {
        let refused = |reason: String| {
            Error::new(
                codes::INBOX_HANDLE_NOT_ALLOWED,
                format!(
                    "'{sender_id}' can't attach conversation file '{}': {reason}",
                    used.file_id
                ),
            )
        };
        if used.conversation_id != conversation_id {
            return Err(refused(format!(
                "it was uploaded to conversation '{}', not to '{conversation_id}'",
                used.conversation_id
            )));
        }
        let row = file_row(db, conversation_id, &used.file_id)?
            .ok_or_else(|| refused("no such file in this conversation".to_string()))?;
        if row.uploader_id != sender_id {
            return Err(refused("only its uploader attaches it".to_string()));
        }
        let ready = row.ready(conversation_id);
        let handle = &used.handle;
        if handle.path != ready.path
            || handle.bytes != ready.bytes
            || handle.sha256 != ready.sha256
            || handle.mime != ready.mime
        {
            return Err(refused(
                "the handle's path, bytes, sha256 and mime must be the ones the upload returned"
                    .to_string(),
            ));
        }
        if row
            .message_id
            .as_deref()
            .is_some_and(|owner| owner != message_id)
        {
            return Err(Error::invalid(format!(
                "File '{}' belongs to another message; upload it again",
                used.file_id
            )));
        }
        db.execute(
            "UPDATE conversation_files SET message_id = ?2 WHERE id = ?1",
            params![used.file_id, message_id],
        )?;
    }
    Ok(())
}

/// The files of a deleted message go with it.
pub(super) fn drop_message(
    db: &Connection,
    changes: &mut Changes,
    conversation_id: &str,
    message_id: &str,
) -> Result<()> {
    db.execute(
        "DELETE FROM conversation_files WHERE conversation_id = ?1 AND message_id = ?2",
        params![conversation_id, message_id],
    )?;
    changes.files();
    Ok(())
}

/// Removes the rows of files nobody sent within a day, then every file on disk that has no row.
pub(crate) fn sweep_files(db: &Connection, dir: &Path) {
    sweep_files_at(db, dir, now_micros());
}

pub(crate) fn sweep_files_at(db: &Connection, dir: &Path, now: i64) {
    let _ = db.execute(
        "DELETE FROM conversation_files WHERE message_id IS NULL AND created_at < ?1",
        [now - UNSENT_GRACE_MICROS],
    );
    let Ok(mut statement) = db.prepare("SELECT conversation_id, id FROM conversation_files") else {
        return;
    };
    let Ok(rows) = statement.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    }) else {
        return;
    };
    let kept: HashSet<(String, String)> = rows.filter_map(|row| row.ok()).collect();
    let Ok(conversations) = std::fs::read_dir(dir) else {
        return;
    };
    for conversation in conversations.flatten() {
        let conversation_path = conversation.path();
        let conversation_id = conversation.file_name().to_string_lossy().to_string();
        let Ok(files) = std::fs::read_dir(&conversation_path) else {
            continue;
        };
        for file in files.flatten() {
            let name = file.file_name().to_string_lossy().to_string();
            let id = name
                .split_once('.')
                .map_or(name.as_str(), |(id, _)| id)
                .to_string();
            if !kept.contains(&(conversation_id.clone(), id)) {
                let _ = std::fs::remove_file(file.path());
            }
        }
        let _ = std::fs::remove_dir(&conversation_path);
    }
}

impl LocalBotStore {
    /// Stores a file `uploader_id` puts into a conversation they are an active member of.
    pub fn store_conversation_file(
        &self,
        conversation_id: &str,
        uploader_id: &str,
        name: &str,
        mime: Option<&str>,
        data: &[u8],
    ) -> Result<LocalConversationFile> {
        safe_component(conversation_id, "conversation id")?;
        let (name, mime) = validate_upload(name, mime, data)?;
        let files_dir = self.files_dir.clone();
        self.write(|tx, _changes| {
            active_membership(tx, conversation_id, uploader_id)?;
            let id = create_id();
            let row = Row {
                id,
                uploader_id: uploader_id.to_string(),
                name,
                mime,
                bytes: data.len() as i64,
                sha256: sha256_hex(data),
                message_id: None,
            };
            let ready = row.ready(conversation_id);
            let location = file_location(&files_dir, conversation_id, &ready.path);
            write_bytes(&location, data)?;
            let inserted = tx.execute(
                "INSERT INTO conversation_files
                 (id, conversation_id, uploader_id, name, mime, bytes, sha256, message_id, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL, ?8)",
                params![
                    row.id,
                    conversation_id,
                    row.uploader_id,
                    row.name,
                    row.mime,
                    row.bytes,
                    row.sha256,
                    now_micros()
                ],
            );
            if let Err(error) = inserted {
                let _ = std::fs::remove_file(&location);
                return Err(error.into());
            }
            Ok(ready)
        })
    }

    /// The file at `path` (its id with the extension) and its bytes, for `principal_id`, who must
    /// be an active member of the conversation. `None` when it does not exist.
    pub fn read_conversation_file(
        &self,
        conversation_id: &str,
        principal_id: &str,
        path: &str,
    ) -> Result<Option<(LocalConversationFile, Vec<u8>)>> {
        safe_component(conversation_id, "conversation id")?;
        let file_id = path.split_once('.').map_or(path, |(id, _)| id);
        safe_component(file_id, "file id")?;
        self.read(|db| {
            active_membership(db, conversation_id, principal_id)?;
            let Some(row) = file_row(db, conversation_id, file_id)? else {
                return Ok(None);
            };
            let ready = row.ready(conversation_id);
            let location = file_location(&self.files_dir, conversation_id, &ready.path);
            match std::fs::read(&location) {
                Ok(bytes) => Ok(Some((ready, bytes))),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
                Err(error) => Err(Error::storage(format!(
                    "Reading conversation file '{file_id}' failed: {error}"
                ))),
            }
        })
    }
}

fn file_location(dir: &Path, conversation_id: &str, stored: &str) -> PathBuf {
    dir.join(conversation_id).join(stored)
}

fn write_bytes(location: &Path, data: &[u8]) -> Result<()> {
    if let Some(parent) = location.parent() {
        std::fs::create_dir_all(parent).map_err(|error| {
            Error::storage(format!("Creating {} failed: {error}", parent.display()))
        })?;
    }
    std::fs::write(location, data)
        .map_err(|error| Error::storage(format!("Writing {} failed: {error}", location.display())))
}

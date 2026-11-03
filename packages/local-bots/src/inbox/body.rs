//! What a plain envelope's body must hold for its kind, through the shared contract the API
//! checks with, and where its workspace files may come from.

use crate::error::{Error, Result, codes};
use chrono::DateTime;
use flow_like_types::{
    bot_message::MessageBody,
    inbox::{InboxEntryKind, TopicBody, TriggerBody},
};
use rusqlite::{Connection, OptionalExtension};
use serde_json::Value;
use std::collections::BTreeSet;

/// Checks a plain envelope's body against its kind: messages and edits carry a [`MessageBody`],
/// topics a [`TopicBody`]. Returns the workspace apps the body's handles name.
pub(super) fn workspace_apps(kind: InboxEntryKind, envelope: &Value) -> Result<BTreeSet<String>> {
    let body = envelope.get("body").unwrap_or(&Value::Null);
    match kind {
        InboxEntryKind::Message | InboxEntryKind::Edit => {
            let message =
                MessageBody::parse(body).map_err(|error| Error::invalid(error.to_string()))?;
            if let Some(ask) = &message.ask
                && DateTime::parse_from_rfc3339(&ask.reply_by).is_err()
            {
                return Err(Error::invalid(format!(
                    "body.ask.reply_by '{}' is not an RFC 3339 time",
                    ask.reply_by
                )));
            }
            Ok(message
                .handles()
                .map(|handle| handle.app_id.clone())
                .collect())
        }
        InboxEntryKind::Topic => TopicBody::parse(body)
            .map(|_| BTreeSet::new())
            .map_err(Error::invalid),
        InboxEntryKind::Trigger => TriggerBody::parse(body)
            .map(|_| BTreeSet::new())
            .map_err(Error::invalid),
        InboxEntryKind::Reaction | InboxEntryKind::Delete | InboxEntryKind::Member => {
            Ok(BTreeSet::new())
        }
    }
}

/// Workspace files travel only from a bot's own workspace (`bots.workspace_app_id`): every app a
/// sender's handles name must be that workspace, so the person never sends one and a bot never
/// points another bot's host at a third app.
pub(super) fn check_workspace_files(
    db: &Connection,
    sender_id: &str,
    apps: &BTreeSet<String>,
) -> Result<()> {
    if apps.is_empty() {
        return Ok(());
    }
    let workspace: Option<String> = db
        .prepare_cached("SELECT workspace_app_id FROM bots WHERE id = ?1")?
        .query_row([sender_id], |row| row.get(0))
        .optional()?
        .flatten();
    match apps
        .iter()
        .find(|app| Some(app.as_str()) != workspace.as_deref())
    {
        Some(stranger) => Err(Error::new(
            codes::INBOX_HANDLE_NOT_ALLOWED,
            format!(
                "'{sender_id}' can't attach files of app '{stranger}': only a bot's host attaches files, and only from that bot's own workspace"
            ),
        )),
        None => Ok(()),
    }
}

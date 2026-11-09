//! The owner's view of a bot's notes: keyed working state the bot keeps in one conversation
//! (see `bot_notes` in the contracts crate). The body is hidden from everyone but the bot and its
//! owner; these routes are the owner's. Every query is scoped to the bot in SQL, and the bot must
//! be or have been a member of the conversation.

use super::owned_bot;
use crate::{
    bot_host::notes::{self, WriteError},
    entity::{conversation, conversation_member},
    error::ApiError,
    middleware::jwt::AppUser,
    routes::inbox::now_millis,
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};
use flow_like_types::{
    bot_notes::{BotNote, MAX_OPEN_NOTES, NoteAuthor, NoteList, NoteWriteRequest, is_note_key},
    inbox::ConversationEncryption,
};
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, PaginatorTrait, QueryFilter};
use serde::Deserialize;
use utoipa::{IntoParams, ToSchema};

#[derive(Debug, Deserialize, IntoParams)]
pub struct ListNotesQuery {
    /// The conversation the notes belong to.
    pub conversation_id: String,
}

/// A previous version of a note.
#[derive(ToSchema)]
#[allow(dead_code)]
pub struct NoteVersionSchema {
    pub version: u32,
    pub title: String,
    pub status: Option<String>,
    pub body: String,
    /// `open` or `closed`.
    pub state: String,
    /// The bot id, or the owner's sub.
    pub updated_by: String,
    /// Epoch milliseconds.
    pub updated_at: i64,
}

/// A note as its owner sees it.
#[derive(ToSchema)]
#[allow(dead_code)]
pub struct NoteSchema {
    /// Lowercase letters, digits and hyphens.
    #[schema(example = "learning-path")]
    pub key: String,
    pub title: String,
    /// One line shown with the title.
    pub status: Option<String>,
    /// Markdown only the bot and its owner read.
    pub body: String,
    /// `open` or `closed`.
    pub state: String,
    pub version: u32,
    /// `{ classification?, pii }`: the most sensitive data the note was written under.
    #[schema(value_type = Object)]
    pub sensitivity: serde_json::Value,
    pub updated_by: String,
    /// Epoch milliseconds.
    pub created_at: i64,
    pub updated_at: i64,
    /// Previous versions, newest first. Left out of lists.
    pub history: Vec<NoteVersionSchema>,
}

#[derive(ToSchema)]
#[allow(dead_code)]
pub struct NoteListSchema {
    pub notes: Vec<NoteSchema>,
}

/// A change to a note; fields left out stay as they are.
#[derive(ToSchema)]
#[allow(dead_code)]
pub struct NoteWriteSchema {
    pub title: Option<String>,
    /// One line; empty clears it.
    pub status: Option<String>,
    pub body: Option<String>,
    /// `open` or `closed`.
    pub state: Option<String>,
    /// Refuses the write when the note moved past this version.
    pub expected_version: Option<u32>,
    /// A desktop host writing for the bot's turn, not the owner editing.
    pub as_bot: bool,
    /// The turn label of a host writing as the bot.
    #[schema(value_type = Object)]
    pub sensitivity: serde_json::Value,
}

fn checked_key(key: &str) -> Result<(), ApiError> {
    if is_note_key(key) {
        return Ok(());
    }
    Err(ApiError::bad_request(
        "A note key is 1 to 48 characters of a-z, 0-9 and '-', starting with a letter or digit",
    ))
}

/// The conversation the bot is or was a member of, readable by the server. Anything else is 404;
/// a sealed conversation holds no notes.
async fn bot_conversation<C: ConnectionTrait>(
    db: &C,
    bot_id: &str,
    conversation_id: &str,
) -> Result<(), ApiError> {
    let not_found = || ApiError::not_found(format!("Conversation '{conversation_id}' not found"));
    let member = conversation_member::Entity::find()
        .filter(conversation_member::Column::ConversationId.eq(conversation_id))
        .filter(conversation_member::Column::PrincipalId.eq(bot_id))
        .count(db)
        .await?;
    let row = conversation::Entity::find_by_id(conversation_id)
        .one(db)
        .await?
        .filter(|_| member > 0)
        .ok_or_else(not_found)?;
    if ConversationEncryption::parse(&row.encryption) == Some(ConversationEncryption::E2e) {
        return Err(ApiError::bad_request(
            "Notes are not kept in end-to-end encrypted conversations",
        ));
    }
    Ok(())
}

fn refused(error: WriteError) -> ApiError {
    match error {
        WriteError::Invalid(error) => ApiError::bad_request(error.to_string()),
        WriteError::TooManyOpen => ApiError::bad_request(format!(
            "A bot keeps at most {MAX_OPEN_NOTES} open notes in one conversation; close one first"
        )),
        WriteError::Conflict { .. } => ApiError::conflict(error.to_string()),
        WriteError::Db(error) => error.into(),
    }
}

#[utoipa::path(
    get,
    path = "/bots/{bot_id}/notes",
    tag = "Bots",
    params(("bot_id" = String, Path, description = "Your bot"), ListNotesQuery),
    responses(
        (status = 200, description = "The notes your bot keeps in this conversation, newest first, without their history.", body = NoteListSchema),
        (status = 400, description = "The conversation is end-to-end encrypted, so it holds no notes"),
        (status = 401, description = "Not signed in"),
        (status = 404, description = "You have no such bot, or it is not in this conversation")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(name = "GET /bots/{bot_id}/notes", skip(state, user))]
pub async fn list_notes(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(bot_id): Path<String>,
    Query(query): Query<ListNotesQuery>,
) -> Result<Json<NoteList>, ApiError> {
    let sub = user.sub()?;
    owned_bot(&state.db, &sub, &bot_id).await?;
    bot_conversation(&state.db, &bot_id, &query.conversation_id).await?;
    let notes = notes::list_in(&state.db, &bot_id, &query.conversation_id).await?;
    Ok(Json(NoteList { notes }))
}

#[utoipa::path(
    get,
    path = "/bots/{bot_id}/notes/{conversation_id}/{key}",
    tag = "Bots",
    params(
        ("bot_id" = String, Path, description = "Your bot"),
        ("conversation_id" = String, Path, description = "The conversation the note belongs to"),
        ("key" = String, Path, description = "The note's key")
    ),
    responses(
        (status = 200, description = "The note with its last versions.", body = NoteSchema),
        (status = 400, description = "The key is invalid, or the conversation is end-to-end encrypted"),
        (status = 401, description = "Not signed in"),
        (status = 404, description = "You have no such bot, conversation or note")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(
    name = "GET /bots/{bot_id}/notes/{conversation_id}/{key}",
    skip(state, user)
)]
pub async fn get_note(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((bot_id, conversation_id, key)): Path<(String, String, String)>,
) -> Result<Json<BotNote>, ApiError> {
    let sub = user.sub()?;
    owned_bot(&state.db, &sub, &bot_id).await?;
    checked_key(&key)?;
    bot_conversation(&state.db, &bot_id, &conversation_id).await?;
    notes::get_in(&state.db, &bot_id, &conversation_id, &key)
        .await?
        .map(Json)
        .ok_or_else(|| ApiError::not_found(format!("Note '{key}' not found")))
}

#[utoipa::path(
    patch,
    path = "/bots/{bot_id}/notes/{conversation_id}/{key}",
    tag = "Bots",
    params(
        ("bot_id" = String, Path, description = "Your bot"),
        ("conversation_id" = String, Path, description = "The conversation the note belongs to"),
        ("key" = String, Path, description = "The note's key")
    ),
    request_body = NoteWriteSchema,
    responses(
        (status = 200, description = "The note after the change; a change that alters nothing returns it as it is.", body = NoteSchema),
        (status = 400, description = "The change is invalid (a new note needs a title; too long; more open notes than allowed), or the conversation is end-to-end encrypted"),
        (status = 401, description = "Not signed in"),
        (status = 404, description = "You have no such bot or conversation"),
        (status = 409, description = "The note moved past expected_version; read it again")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(
    name = "PATCH /bots/{bot_id}/notes/{conversation_id}/{key}",
    skip(state, user, body)
)]
pub async fn write_note(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((bot_id, conversation_id, key)): Path<(String, String, String)>,
    Json(body): Json<NoteWriteRequest>,
) -> Result<Json<BotNote>, ApiError> {
    let sub = user.sub()?;
    owned_bot(&state.db, &sub, &bot_id).await?;
    checked_key(&key)?;
    bot_conversation(&state.db, &bot_id, &conversation_id).await?;
    let author = if body.as_bot {
        NoteAuthor {
            by: bot_id.clone(),
            at: now_millis().timestamp_millis(),
            sensitivity: body.sensitivity,
        }
    } else {
        NoteAuthor {
            by: sub,
            at: now_millis().timestamp_millis(),
            sensitivity: Default::default(),
        }
    };
    notes::write_in(
        &state.db,
        &bot_id,
        &conversation_id,
        &key,
        &body.change,
        &author,
        body.expected_version,
    )
    .await
    .map(Json)
    .map_err(refused)
}

//! The owner's view of a bot's mail: its threads and the mails in each. A thread's mails are
//! found through its `BotMailMessage` rows, wherever they were written (the owner's direct
//! conversation with the bot, or the thread's own group). A mail counts as read once the owner
//! read the conversation that holds it.

use super::owned_bot;
use crate::{
    entity::{bot_mail_message, bot_mail_thread, conversation_member, inbox_entry},
    error::ApiError,
    middleware::jwt::AppUser,
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};
use chrono::{DateTime, FixedOffset};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use utoipa::{IntoParams, ToSchema};

const PAGE: u64 = 50;
const MAX_MAILS: u64 = 200;
const UNREAD_SCAN: u64 = 500;

/// One mail thread of a bot.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct MailThread {
    /// The thread's id.
    pub id: String,
    pub subject: String,
    /// Lowercase correspondent addresses.
    pub participants: Vec<String>,
    pub last_message_at: DateTime<FixedOffset>,
    pub last_snippet: Option<String>,
    pub message_count: i32,
    /// Received mails the owner has not read yet.
    pub unread: u64,
    pub created_at: DateTime<FixedOffset>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct MailThreadList {
    /// Newest activity first.
    pub threads: Vec<MailThread>,
}

#[derive(Debug, Deserialize, IntoParams)]
pub struct ThreadsQuery {
    /// Only threads with activity before this time (RFC 3339), for the next page.
    pub before: Option<DateTime<FixedOffset>>,
}

/// One mail of a thread.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct MailItem {
    pub id: String,
    pub seq: Option<i64>,
    /// `in` or `out`.
    pub direction: String,
    pub at: DateTime<FixedOffset>,
    #[schema(value_type = Object)]
    pub from: Value,
    #[schema(value_type = Object)]
    pub to: Value,
    #[schema(value_type = Object)]
    pub cc: Value,
    pub subject: String,
    pub text: String,
    /// False when the sender could not be verified.
    pub authenticated: bool,
    /// File names, types, sizes and workspace handles; a handle opens the file in Files.
    #[schema(value_type = Object)]
    pub attachments: Value,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct MailThreadView {
    pub thread: MailThread,
    pub mails: Vec<MailItem>,
}

fn participants_of(row: &bot_mail_thread::Model) -> Vec<String> {
    serde_json::from_str(&row.participants).unwrap_or_default()
}

fn view(row: bot_mail_thread::Model, unread: u64) -> MailThread {
    MailThread {
        participants: participants_of(&row),
        id: row.id,
        subject: row.subject,
        last_message_at: row.last_message_at,
        last_snippet: row.last_snippet,
        message_count: row.message_count,
        unread,
        created_at: row.created_at,
    }
}

/// The owner's copies of these entries, oldest first.
async fn owner_entries(
    state: &AppState,
    owner: &str,
    message_ids: Vec<String>,
) -> Result<Vec<inbox_entry::Model>, ApiError> {
    if message_ids.is_empty() {
        return Ok(Vec::new());
    }
    Ok(inbox_entry::Entity::find()
        .filter(inbox_entry::Column::InboxId.eq(owner))
        .filter(inbox_entry::Column::MessageId.is_in(message_ids))
        .order_by_asc(inbox_entry::Column::CreatedAt)
        .order_by_asc(inbox_entry::Column::Id)
        .all(&state.db)
        .await?)
}

/// The owner's copies of one thread's mails, oldest first.
async fn thread_entries(
    state: &AppState,
    owner: &str,
    bot_id: &str,
    thread_id: &str,
) -> Result<Vec<inbox_entry::Model>, ApiError> {
    let mail_ids: Vec<String> = bot_mail_message::Entity::find()
        .select_only()
        .column(bot_mail_message::Column::EntryMessageId)
        .filter(bot_mail_message::Column::ThreadId.eq(thread_id))
        .filter(bot_mail_message::Column::BotId.eq(bot_id))
        .order_by_asc(bot_mail_message::Column::CreatedAt)
        .limit(MAX_MAILS)
        .into_tuple()
        .all(&state.db)
        .await?;
    owner_entries(state, owner, mail_ids).await
}

/// When the owner last read each of the conversations that hold these entries.
async fn read_markers(
    state: &AppState,
    owner: &str,
    entries: &[inbox_entry::Model],
) -> Result<HashMap<String, Option<DateTime<FixedOffset>>>, ApiError> {
    let holding: HashSet<String> = entries
        .iter()
        .map(|entry| entry.conversation_id.clone())
        .collect();
    if holding.is_empty() {
        return Ok(HashMap::new());
    }
    Ok(conversation_member::Entity::find()
        .filter(conversation_member::Column::PrincipalId.eq(owner))
        .filter(conversation_member::Column::ConversationId.is_in(holding))
        .all(&state.db)
        .await?
        .into_iter()
        .map(|member| (member.conversation_id, member.last_read_at))
        .collect())
}

/// Received mails per thread that are newer than the owner's read marker of the conversation
/// holding them; only the newest [`UNREAD_SCAN`] received mails of the page are counted.
async fn unread_counts(
    state: &AppState,
    owner: &str,
    ids: &[String],
) -> Result<HashMap<String, u64>, ApiError> {
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    let received: Vec<(String, String)> = bot_mail_message::Entity::find()
        .select_only()
        .column(bot_mail_message::Column::ThreadId)
        .column(bot_mail_message::Column::EntryMessageId)
        .filter(bot_mail_message::Column::ThreadId.is_in(ids.iter().cloned()))
        .filter(bot_mail_message::Column::Direction.eq("IN"))
        .order_by_desc(bot_mail_message::Column::CreatedAt)
        .limit(UNREAD_SCAN)
        .into_tuple()
        .all(&state.db)
        .await?;
    let entries = owner_entries(
        state,
        owner,
        received.iter().map(|(_, entry)| entry.clone()).collect(),
    )
    .await?;
    let read = read_markers(state, owner, &entries).await?;
    let unread: HashSet<&str> = entries
        .iter()
        .filter(|entry| {
            read.get(&entry.conversation_id)
                .copied()
                .flatten()
                .is_none_or(|seen| entry.created_at > seen)
        })
        .map(|entry| entry.message_id.as_str())
        .collect();
    let mut counts: HashMap<String, u64> = HashMap::new();
    for (thread, entry) in &received {
        if unread.contains(entry.as_str()) {
            *counts.entry(thread.clone()).or_default() += 1;
        }
    }
    Ok(counts)
}

#[utoipa::path(
    get,
    path = "/bots/{bot_id}/mail/threads",
    tag = "Bots",
    description = "List the bot's mail threads, newest activity first, with correspondents, the last message and how many received mails you have not read.",
    params(("bot_id" = String, Path, description = "Your bot's id"), ThreadsQuery),
    responses(
        (status = 200, description = "The bot's mail threads", body = MailThreadList),
        (status = 401, description = "Not signed in"),
        (status = 404, description = "You have no bot with this id")
    ),
    security(("bearer_auth" = []))
)]
pub async fn list_mail_threads(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(bot_id): Path<String>,
    Query(query): Query<ThreadsQuery>,
) -> Result<Json<MailThreadList>, ApiError> {
    let sub = user.sub()?;
    let bot = owned_bot(&state.db, &sub, &bot_id).await?;
    let mut select = bot_mail_thread::Entity::find()
        .filter(bot_mail_thread::Column::BotId.eq(bot.id.as_str()))
        .filter(bot_mail_thread::Column::OwnerId.eq(sub.as_str()));
    if let Some(before) = query.before {
        select = select.filter(bot_mail_thread::Column::LastMessageAt.lt(before));
    }
    let rows = select
        .order_by_desc(bot_mail_thread::Column::LastMessageAt)
        .limit(PAGE)
        .all(&state.db)
        .await?;
    let ids: Vec<String> = rows.iter().map(|row| row.id.clone()).collect();
    let counts = unread_counts(&state, &sub, &ids).await?;
    let threads = rows
        .into_iter()
        .map(|row| {
            let unread = counts.get(&row.id).copied().unwrap_or(0);
            view(row, unread)
        })
        .collect();
    Ok(Json(MailThreadList { threads }))
}

fn text_of(body: &Value) -> String {
    body.get("text")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

pub(crate) fn mail_item(entry: &inbox_entry::Model) -> Option<MailItem> {
    let body = entry.envelope.get("body")?;
    let (direction, source) = match entry.kind.as_str() {
        "TRIGGER" if body.get("source").and_then(Value::as_str) == Some("email") => {
            ("in", body.get("payload")?)
        }
        "MESSAGE" => ("out", body.get("mail")?),
        _ => return None,
    };
    let field = |name: &str| source.get(name).cloned().unwrap_or(Value::Null);
    let text = if direction == "in" {
        source
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned()
    } else {
        text_of(body)
    };
    Some(MailItem {
        id: entry.message_id.clone(),
        seq: entry.seq,
        direction: direction.to_owned(),
        at: entry.created_at,
        from: if direction == "out" {
            Value::String(source.get("from")?.as_str()?.to_owned())
        } else {
            field("from")
        },
        to: field("to"),
        cc: field("cc"),
        subject: source
            .get("subject")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        text,
        authenticated: direction == "out"
            || source
                .get("authenticated")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        attachments: field("attachments"),
    })
}

#[utoipa::path(
    get,
    path = "/bots/{bot_id}/mail/threads/{thread_id}",
    tag = "Bots",
    description = "Open one mail thread: every mail the bot received and sent in it, oldest first.",
    params(
        ("bot_id" = String, Path, description = "Your bot's id"),
        ("thread_id" = String, Path, description = "The thread's id")
    ),
    responses(
        (status = 200, description = "The thread and its mails", body = MailThreadView),
        (status = 401, description = "Not signed in"),
        (status = 404, description = "No such bot or thread of yours")
    ),
    security(("bearer_auth" = []))
)]
pub async fn get_mail_thread(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((bot_id, thread_id)): Path<(String, String)>,
) -> Result<Json<MailThreadView>, ApiError> {
    let sub = user.sub()?;
    let bot = owned_bot(&state.db, &sub, &bot_id).await?;
    let row = bot_mail_thread::Entity::find_by_id(thread_id.as_str())
        .filter(bot_mail_thread::Column::BotId.eq(bot.id.as_str()))
        .filter(bot_mail_thread::Column::OwnerId.eq(sub.as_str()))
        .one(&state.db)
        .await?
        .ok_or_else(|| ApiError::not_found("Mail thread not found"))?;
    let entries = thread_entries(&state, &sub, &bot.id, &row.id).await?;
    let counts = unread_counts(&state, &sub, std::slice::from_ref(&row.id)).await?;
    let unread = counts.get(&row.id).copied().unwrap_or(0);
    Ok(Json(MailThreadView {
        thread: view(row, unread),
        mails: entries.iter().filter_map(mail_item).collect(),
    }))
}

/// A mail a desktop-hosted account bot sends through the hub, which holds the mail provider.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct BotSendMail {
    pub to: Vec<String>,
    #[serde(default)]
    pub cc: Vec<String>,
    pub subject: String,
    pub text: String,
    pub html: String,
    /// Files with `filename`, `content_type` and standard-base64 `content`; at most 10, 3 MiB.
    #[serde(default)]
    #[schema(value_type = Vec<Object>)]
    pub attachments: Vec<flow_like_catalog_core::MailAttachment>,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct BotReplyMail {
    pub thread: String,
    pub text: String,
    pub html: String,
    #[serde(default)]
    #[schema(value_type = Vec<Object>)]
    pub attachments: Vec<flow_like_catalog_core::MailAttachment>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct BotSentMail {
    pub message_id: String,
    pub thread: String,
    pub from: String,
    pub to: Vec<String>,
    pub subject: String,
}

fn files_of(
    attachments: Vec<flow_like_catalog_core::MailAttachment>,
) -> Result<Vec<flow_like_bot_tools::email::MailFile>, ApiError> {
    use base64::{Engine, engine::general_purpose::STANDARD};
    attachments
        .into_iter()
        .map(|attachment| {
            Ok(flow_like_bot_tools::email::MailFile {
                bytes: STANDARD
                    .decode(&attachment.content)
                    .map_err(|_| {
                        ApiError::bad_request(format!(
                            "Attachment {:?} content is not standard base64",
                            attachment.filename
                        ))
                    })?
                    .into(),
                filename: attachment.filename,
                content_type: attachment.content_type,
            })
        })
        .collect()
}

pub(crate) const MAIL_LIMIT: &str = "MAIL_LIMIT";
pub(crate) const MAIL_NO_THREAD: &str = "MAIL_NO_THREAD";
pub(crate) const MAIL_NO_ADDRESS: &str = "MAIL_NO_ADDRESS";

pub(crate) fn failure_status(failure: flow_like_bot_tools::email::MailFailure) -> ApiError {
    use flow_like_bot_tools::email::failure as code;
    match failure.code {
        code::LIMIT => ApiError::coded(
            axum::http::StatusCode::TOO_MANY_REQUESTS,
            MAIL_LIMIT,
            failure.message,
        ),
        code::NO_THREAD => ApiError::coded(
            axum::http::StatusCode::NOT_FOUND,
            MAIL_NO_THREAD,
            failure.message,
        ),
        code::NO_ADDRESS => ApiError::coded(
            axum::http::StatusCode::BAD_REQUEST,
            MAIL_NO_ADDRESS,
            failure.message,
        ),
        code::REFUSED => ApiError::bad_request(failure.message),
        _ => ApiError::service_unavailable(failure.message),
    }
}

fn sent_view(sent: flow_like_bot_tools::email::SentMail) -> Json<BotSentMail> {
    Json(BotSentMail {
        message_id: sent.message_id,
        thread: sent.thread,
        from: sent.from,
        to: sent.to,
        subject: sent.subject,
    })
}

#[utoipa::path(
    post,
    path = "/bots/{bot_id}/mail/send",
    tag = "Bots",
    description = "Send an email from the bot's own address, for a bot whose host runs on your computer. The hub holds the mail provider, applies the daily and per-interval limits and records the mail in a thread.",
    params(("bot_id" = String, Path, description = "Your bot's id")),
    request_body = BotSendMail,
    responses(
        (status = 200, description = "The mail was accepted by the provider", body = BotSentMail),
        (status = 400, description = "Invalid mail (BAD_REQUEST), or the bot has no address (MAIL_NO_ADDRESS)"),
        (status = 404, description = "You have no bot with this id"),
        (status = 429, description = "MAIL_LIMIT: the mail limit is reached"),
        (status = 503, description = "Mail is unavailable")
    ),
    security(("bearer_auth" = []))
)]
pub async fn send_bot_mail(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(bot_id): Path<String>,
    Json(request): Json<BotSendMail>,
) -> Result<Json<BotSentMail>, ApiError> {
    let sub = crate::bot_caller::acting_owner_for(&state.db, &user, &bot_id).await?;
    let bot = owned_bot(&state.db, &sub, &bot_id).await?;
    let mail = flow_like_bot_tools::email::OutgoingMail {
        to: request.to,
        cc: request.cc,
        subject: request.subject,
        text: request.text,
        html: request.html,
        attachments: files_of(request.attachments)?,
    };
    crate::bot_mail::send(&state, &bot.id, mail)
        .await
        .map(sent_view)
        .map_err(failure_status)
}

#[utoipa::path(
    post,
    path = "/bots/{bot_id}/mail/reply",
    tag = "Bots",
    description = "Reply in one of the bot's mail threads, to the sender of its latest received mail, for a bot whose host runs on your computer.",
    params(("bot_id" = String, Path, description = "Your bot's id")),
    request_body = BotReplyMail,
    responses(
        (status = 200, description = "The reply was accepted by the provider", body = BotSentMail),
        (status = 400, description = "Invalid mail (BAD_REQUEST), or the bot has no address (MAIL_NO_ADDRESS)"),
        (status = 404, description = "No such bot of yours, or MAIL_NO_THREAD: no such mail thread"),
        (status = 429, description = "MAIL_LIMIT: the mail limit is reached"),
        (status = 503, description = "Mail is unavailable")
    ),
    security(("bearer_auth" = []))
)]
pub async fn reply_bot_mail(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(bot_id): Path<String>,
    Json(request): Json<BotReplyMail>,
) -> Result<Json<BotSentMail>, ApiError> {
    let sub = crate::bot_caller::acting_owner_for(&state.db, &user, &bot_id).await?;
    let bot = owned_bot(&state.db, &sub, &bot_id).await?;
    let mail = flow_like_bot_tools::email::ReplyMail {
        thread: request.thread,
        text: request.text,
        html: request.html,
        attachments: files_of(request.attachments)?,
    };
    crate::bot_mail::reply(&state, &bot.id, mail)
        .await
        .map(sent_view)
        .map_err(failure_status)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::StatusCode;
    use flow_like_bot_tools::email::{MailFailure, failure};

    #[test]
    fn mail_refusals_carry_their_own_public_codes() {
        let cases = [
            (failure::LIMIT, StatusCode::TOO_MANY_REQUESTS, "MAIL_LIMIT"),
            (failure::NO_THREAD, StatusCode::NOT_FOUND, "MAIL_NO_THREAD"),
            (
                failure::NO_ADDRESS,
                StatusCode::BAD_REQUEST,
                "MAIL_NO_ADDRESS",
            ),
        ];
        for (kind, status, public) in cases {
            let error = failure_status(MailFailure::new(kind, "refused"));
            assert_eq!((error.status(), error.public_code()), (status, public));
        }
        let refused = failure_status(MailFailure::new(failure::REFUSED, "refused"));
        assert_eq!(refused.status(), StatusCode::BAD_REQUEST);
    }
}

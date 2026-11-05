//! Stars: a person keeps messages for later, across conversations, and only they see them.
//! Starring again changes nothing; deleting the message removes every star of it.

use super::{
    active_member, active_membership,
    marks::{copies_of, latest_edits, live_copy},
    model::InboxEntry,
    now_millis, wake,
};
use crate::{
    entity::{conversation_member, inbox_entry, starred_message},
    error::ApiError,
    middleware::jwt::AppUser,
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use chrono::{DateTime, FixedOffset};
use flow_like_types::inbox::{EntryCursor, is_tombstone};
use sea_orm::{
    ActiveValue::Set, ColumnTrait, Condition, ConnectionTrait, EntityTrait, QueryFilter,
    QueryOrder, QuerySelect, QueryTrait, sea_query::OnConflict,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use utoipa::{IntoParams, ToSchema};

const DEFAULT_STARS_PAGE: u64 = 50;
const MAX_STARS_PAGE: u64 = 200;

#[derive(Debug, Clone, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct StarsQuery {
    /// Only the stars of this conversation.
    pub conversation_id: Option<String>,
    /// Only stars older than this cursor.
    pub before: Option<String>,
    /// 1 to 200 stars; 50 by default.
    pub limit: Option<u64>,
}

/// A message you starred.
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub struct StarredMessage {
    pub conversation_id: String,
    pub message_id: String,
    pub starred_at: DateTime<FixedOffset>,
    /// Your copy of the message.
    pub entry: InboxEntry,
    /// The sender's newest edit of it in your copy.
    pub latest_edit: Option<InboxEntry>,
    /// Opaque position; pass it back as `before`.
    pub cursor: String,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct StarredMessageList {
    /// Newest star first. A page may hold fewer than `limit` stars while `next_before` is set.
    pub stars: Vec<StarredMessage>,
    pub next_before: Option<String>,
}

fn cursor_of(star: &starred_message::Model) -> String {
    EntryCursor::new(star.starred_at.timestamp_micros(), star.message_id.as_str()).to_string()
}

fn older_than(before: Option<&str>) -> Result<Option<Condition>, ApiError> {
    let Some(before) = before.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    let cursor = EntryCursor::parse(before)
        .map_err(|error| ApiError::bad_request(format!("Invalid before: {error}")))?;
    let at = DateTime::from_timestamp_micros(cursor.created_at_micros)
        .ok_or_else(|| {
            ApiError::bad_request(format!(
                "Invalid before: {} is outside the supported time range",
                cursor.created_at_micros
            ))
        })?
        .fixed_offset();
    Ok(Some(
        Condition::any()
            .add(starred_message::Column::StarredAt.lt(at))
            .add(
                Condition::all()
                    .add(starred_message::Column::StarredAt.eq(at))
                    .add(starred_message::Column::MessageId.lt(cursor.entry_id)),
            ),
    ))
}

async fn present<C: ConnectionTrait>(
    db: &C,
    sub: &str,
    stars: Vec<starred_message::Model>,
) -> Result<Vec<StarredMessage>, ApiError> {
    let ids: Vec<String> = stars.iter().map(|star| star.message_id.clone()).collect();
    let mut copies: HashMap<String, inbox_entry::Model> = copies_of(db, sub, &ids)
        .await?
        .into_iter()
        .filter(|copy| !is_tombstone(&copy.envelope))
        .map(|copy| (copy.message_id.clone(), copy))
        .collect();
    let mut edits = latest_edits(db, sub, &copies.values().collect::<Vec<_>>()).await?;
    Ok(stars
        .into_iter()
        .filter_map(|star| {
            let copy = copies
                .remove(&star.message_id)
                .filter(|copy| copy.conversation_id == star.conversation_id)?;
            Some(StarredMessage {
                cursor: cursor_of(&star),
                latest_edit: edits.remove(&star.message_id),
                entry: copy.into(),
                conversation_id: star.conversation_id,
                message_id: star.message_id,
                starred_at: star.starred_at,
            })
        })
        .collect())
}

/// The person's stars in conversations they are still an active member of, newest first.
pub(crate) async fn stars_for<C: ConnectionTrait>(
    db: &C,
    sub: &str,
    query: &StarsQuery,
) -> Result<StarredMessageList, ApiError> {
    let limit = query
        .limit
        .unwrap_or(DEFAULT_STARS_PAGE)
        .clamp(1, MAX_STARS_PAGE);
    let conversation_id = query
        .conversation_id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty());
    let mut find = starred_message::Entity::find()
        .filter(starred_message::Column::PrincipalId.eq(sub))
        .order_by_desc(starred_message::Column::StarredAt)
        .order_by_desc(starred_message::Column::MessageId)
        .limit(limit + 1);
    if let Some(conversation_id) = conversation_id {
        active_membership(db, conversation_id, sub).await?;
        find = find.filter(starred_message::Column::ConversationId.eq(conversation_id));
    } else {
        let member_of = conversation_member::Entity::find()
            .select_only()
            .column(conversation_member::Column::ConversationId)
            .filter(conversation_member::Column::PrincipalId.eq(sub))
            .filter(active_member())
            .into_query();
        find = find.filter(starred_message::Column::ConversationId.in_subquery(member_of));
    }
    if let Some(older) = older_than(query.before.as_deref())? {
        find = find.filter(older);
    }
    let mut stars = find.all(db).await?;
    let next_before = if stars.len() as u64 > limit {
        stars.truncate(limit as usize);
        stars.last().map(cursor_of)
    } else {
        None
    };
    Ok(StarredMessageList {
        stars: present(db, sub, stars).await?,
        next_before,
    })
}

pub(crate) async fn star_message_in<C: ConnectionTrait>(
    db: &C,
    conversation_id: &str,
    sub: &str,
    message_id: &str,
    now: DateTime<FixedOffset>,
) -> Result<StarredMessage, ApiError> {
    active_membership(db, conversation_id, sub).await?;
    live_copy(db, conversation_id, sub, message_id).await?;
    starred_message::Entity::insert(starred_message::ActiveModel {
        principal_id: Set(sub.to_string()),
        message_id: Set(message_id.to_string()),
        conversation_id: Set(conversation_id.to_string()),
        starred_at: Set(now),
    })
    .on_conflict(
        OnConflict::columns([
            starred_message::Column::PrincipalId,
            starred_message::Column::MessageId,
        ])
        .do_nothing()
        .to_owned(),
    )
    .exec_without_returning(db)
    .await?;
    let star = starred_message::Entity::find_by_id((sub.to_string(), message_id.to_string()))
        .one(db)
        .await?;
    present(db, sub, star.into_iter().collect())
        .await?
        .pop()
        .ok_or_else(|| {
            ApiError::internal_error(flow_like_types::anyhow!(
                "Starred '{message_id}' in conversation '{conversation_id}' but could not read the star back"
            ))
        })
}

pub(crate) async fn unstar_message_in<C: ConnectionTrait>(
    db: &C,
    conversation_id: &str,
    sub: &str,
    message_id: &str,
) -> Result<bool, ApiError> {
    Ok(starred_message::Entity::delete_many()
        .filter(starred_message::Column::PrincipalId.eq(sub))
        .filter(starred_message::Column::MessageId.eq(message_id))
        .filter(starred_message::Column::ConversationId.eq(conversation_id))
        .exec(db)
        .await?
        .rows_affected
        > 0)
}

#[utoipa::path(
    get,
    path = "/inbox/stars",
    tag = "Inbox",
    params(StarsQuery),
    responses(
        (status = 200, description = "Messages you starred, newest star first, from conversations you are still in. Only you see your stars.", body = StarredMessageList),
        (status = 400, description = "The cursor is malformed"),
        (status = 401, description = "Not signed in"),
        (status = 403, description = "Only people star messages, not bots, API keys or running flows"),
        (status = 404, description = "You are not in the conversation you named")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(name = "GET /inbox/stars", skip(state, user))]
pub async fn list_stars(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Query(query): Query<StarsQuery>,
) -> Result<Json<StarredMessageList>, ApiError> {
    let sub = user.sub()?;
    Ok(Json(stars_for(&state.db, &sub, &query).await?))
}

#[utoipa::path(
    put,
    path = "/inbox/conversations/{conversation_id}/stars/{message_id}",
    tag = "Inbox",
    params(
        ("conversation_id" = String, Path, description = "The conversation"),
        ("message_id" = String, Path, description = "The message to star")
    ),
    responses(
        (status = 200, description = "The message is starred for you; starring it again changes nothing", body = StarredMessage),
        (status = 400, description = "The message id is malformed"),
        (status = 401, description = "Not signed in"),
        (status = 403, description = "Only people star messages, not bots, API keys or running flows"),
        (status = 404, description = "You are not in this conversation"),
        (status = 422, description = "The message isn't in your copy of the conversation, or was deleted (`INBOX_TARGET_NOT_FOUND`)")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(
    name = "PUT /inbox/conversations/{conversation_id}/stars/{message_id}",
    skip(state, user)
)]
pub async fn star_message(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((conversation_id, message_id)): Path<(String, String)>,
) -> Result<Json<StarredMessage>, ApiError> {
    let sub = user.sub()?;
    let star =
        star_message_in(&state.db, &conversation_id, &sub, &message_id, now_millis()).await?;
    wake(&state, [sub]).await;
    Ok(Json(star))
}

#[utoipa::path(
    delete,
    path = "/inbox/conversations/{conversation_id}/stars/{message_id}",
    tag = "Inbox",
    params(
        ("conversation_id" = String, Path, description = "The conversation"),
        ("message_id" = String, Path, description = "The starred message")
    ),
    responses(
        (status = 204, description = "The message is no longer starred for you; unstarring a message you never starred changes nothing"),
        (status = 401, description = "Not signed in"),
        (status = 403, description = "Only people star messages, not bots, API keys or running flows")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(
    name = "DELETE /inbox/conversations/{conversation_id}/stars/{message_id}",
    skip(state, user)
)]
pub async fn unstar_message(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((conversation_id, message_id)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    let sub = user.sub()?;
    if unstar_message_in(&state.db, &conversation_id, &sub, &message_id).await? {
        wake(&state, [sub]).await;
    }
    Ok(StatusCode::NO_CONTENT)
}

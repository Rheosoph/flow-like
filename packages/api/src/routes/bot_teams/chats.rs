//! A person's chat with another owner's bot of an org they share. The person asks, the bot's
//! owner approves with room limits, and the chat is a group conversation of the person, the bot
//! (which reads along) and the owner (who only watches). It ends when either side ends it or
//! when the bot or the person leaves the common org; the person keeps the history.

use super::{
    access::{TEAM_LIMIT, is_active_in, limit, org_of},
    effects::{Effects, settle},
    levels::person_level,
    model::{AcceptChatRequest, ChatRequestBody},
    room::{bot as bot_principal, person, room_limits},
    threads::{announce_watcher, classified, seat_members},
};
use crate::{
    audit,
    bot_host::teams::active_team_of,
    bot_notify::display_name,
    entity::{bot, bot_team, bot_team_chat},
    error::ApiError,
    middleware::jwt::AppUser,
    routes::{
        bots::lock_bot,
        inbox::{
            invitations::validate_limits,
            now_millis,
            team_request::person_label,
            team_room::{RoomJoin, create_room, lock_room},
        },
    },
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
    http::StatusCode,
};
use chrono::{DateTime, Duration, FixedOffset};
use flow_like_types::{
    bot_effective::BotRoomLimits,
    bot_team::{BotChat, BotChatStatus, CHAT_RETRY_DAYS, MAX_CHAT_NOTE_CHARS, person_may_talk},
    create_id,
    data_classification::DataClassification,
};
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, Condition, ConnectionTrait,
    DatabaseTransaction, EntityTrait, PaginatorTrait, QueryFilter,
};

pub(crate) const TEAM_CHAT_LEAD: &str = "TEAM_CHAT_LEAD";
pub(crate) const TEAM_CHAT_EXISTS: &str = "TEAM_CHAT_EXISTS";
pub(crate) const TEAM_CHAT_DECLINED: &str = "TEAM_CHAT_DECLINED";

/// Chat requests one person may have waiting for an answer.
pub(crate) const MAX_PENDING_CHATS: u64 = 10;

pub(crate) fn chat_view(row: &bot_team_chat::Model) -> BotChat {
    BotChat {
        bot_id: row.bot_id.clone(),
        user_id: row.user_id.clone(),
        owner_id: row.owner_id.clone(),
        status: BotChatStatus::parse(&row.status).unwrap_or(BotChatStatus::Ended),
        conversation_id: row.conversation_id.clone(),
        note: row.note.clone(),
        requested_at: row.requested_at.to_rfc3339(),
        decided_at: row.decided_at.map(|at| at.to_rfc3339()),
    }
}

fn status_of(row: &bot_team_chat::Model) -> BotChatStatus {
    BotChatStatus::parse(&row.status).unwrap_or(BotChatStatus::Ended)
}

fn unknown_bot(bot_id: &str) -> ApiError {
    ApiError::not_found(format!("Bot '{bot_id}' not found"))
}

fn unknown_chat(bot_id: &str, user_id: &str) -> ApiError {
    ApiError::not_found(format!(
        "No chat between bot '{bot_id}' and person '{user_id}' found for you"
    ))
}

fn bot_level(bot: &bot::Model) -> Option<DataClassification> {
    bot.classification
        .as_deref()
        .and_then(DataClassification::parse)
}

async fn chat_row<C: ConnectionTrait>(
    db: &C,
    bot_id: &str,
    user_id: &str,
) -> Result<Option<bot_team_chat::Model>, sea_orm::DbErr> {
    bot_team_chat::Entity::find_by_id((bot_id.to_string(), user_id.to_string()))
        .one(db)
        .await
}

async fn live_bot<C: ConnectionTrait>(
    db: &C,
    bot_id: &str,
) -> Result<Option<bot::Model>, sea_orm::DbErr> {
    bot::Entity::find_by_id(bot_id)
        .filter(bot::Column::DeletedAt.is_null())
        .one(db)
        .await
}

/// The team the bot is active in and the org of it, when `user_id` is an active person of that
/// org: where a chat between them may exist.
async fn shared_team<C: ConnectionTrait>(
    db: &C,
    bot_id: &str,
    user_id: &str,
) -> Result<Option<(bot_team::Model, super::access::Org)>, ApiError> {
    let Some(team) = active_team_of(db, bot_id).await? else {
        return Ok(None);
    };
    let org = org_of(db, &team).await?;
    Ok(is_active_in(db, &org.ids(), user_id)
        .await?
        .then_some((team, org)))
}

async fn ensure_level<C: ConnectionTrait>(
    db: &C,
    org_id: &str,
    user_id: &str,
    bot: &bot::Model,
) -> Result<(), ApiError> {
    let level = person_level(db, org_id, user_id).await?;
    let bot_level = bot_level(bot);
    if person_may_talk(Some(level), bot_level) {
        return Ok(());
    }
    Err(classified(&display_name(bot), bot_level))
}

fn ensure_not_lead(team: &bot_team::Model, bot: &bot::Model) -> Result<(), ApiError> {
    if team.lead_bot_id.as_deref() != Some(bot.id.as_str()) {
        return Ok(());
    }
    Err(ApiError::coded(
        StatusCode::CONFLICT,
        TEAM_CHAT_LEAD,
        format!(
            "'{}' leads '{}': talk to it in your thread with the team",
            display_name(bot),
            team.name
        ),
    ))
}

fn trimmed_note(note: Option<&str>) -> Result<Option<String>, ApiError> {
    let note = note.map(str::trim).filter(|note| !note.is_empty());
    if let Some(note) = note
        && note.chars().count() > MAX_CHAT_NOTE_CHARS
    {
        return Err(ApiError::bad_request(format!(
            "The note may have at most {MAX_CHAT_NOTE_CHARS} characters; yours has {}",
            note.chars().count()
        )));
    }
    Ok(note.map(str::to_string))
}

async fn ensure_pending_capacity<C: ConnectionTrait>(
    db: &C,
    user_id: &str,
) -> Result<(), ApiError> {
    let waiting = bot_team_chat::Entity::find()
        .filter(bot_team_chat::Column::UserId.eq(user_id))
        .filter(bot_team_chat::Column::Status.eq(BotChatStatus::Pending.as_str()))
        .count(db)
        .await?;
    if waiting >= MAX_PENDING_CHATS {
        return Err(limit(format!(
            "You can wait for at most {MAX_PENDING_CHATS} chat requests; {waiting} are open ({TEAM_LIMIT})"
        )));
    }
    Ok(())
}

/// What an earlier row for the same bot and person allows: the row to return, or the row to ask
/// again with.
enum Earlier {
    Fresh,
    Pending(bot_team_chat::Model),
    Renew(bot_team_chat::Model),
}

fn earlier(
    row: Option<bot_team_chat::Model>,
    bot: &bot::Model,
    now: DateTime<FixedOffset>,
) -> Result<Earlier, ApiError> {
    let Some(row) = row else {
        return Ok(Earlier::Fresh);
    };
    match status_of(&row) {
        BotChatStatus::Active => Err(ApiError::coded(
            StatusCode::CONFLICT,
            TEAM_CHAT_EXISTS,
            format!("You already talk with '{}'", display_name(bot)),
        )),
        BotChatStatus::Pending => Ok(Earlier::Pending(row)),
        BotChatStatus::Declined => {
            let retry_at =
                row.decided_at.unwrap_or(row.requested_at) + Duration::days(CHAT_RETRY_DAYS);
            if retry_at > now {
                return Err(ApiError::coded(
                    StatusCode::CONFLICT,
                    TEAM_CHAT_DECLINED,
                    format!(
                        "The owner of '{}' declined your request; you may ask again from {}",
                        display_name(bot),
                        retry_at.format("%Y-%m-%d")
                    ),
                ));
            }
            Ok(Earlier::Renew(row))
        }
        BotChatStatus::Ended => Ok(Earlier::Renew(row)),
    }
}

/// Asks the owner of `body.bot_id` for a chat with the caller.
pub(crate) async fn request_chat_in<C: ConnectionTrait>(
    db: &C,
    sub: &str,
    body: &ChatRequestBody,
    now: DateTime<FixedOffset>,
) -> Result<(BotChat, Effects), ApiError> {
    let target = live_bot(db, &body.bot_id)
        .await?
        .ok_or_else(|| unknown_bot(&body.bot_id))?;
    let (team, org) = shared_team(db, &target.id, sub)
        .await?
        .ok_or_else(|| unknown_bot(&body.bot_id))?;
    if target.owner_id == sub {
        return Err(ApiError::bad_request(format!(
            "'{}' is your own bot: talk to it directly",
            display_name(&target)
        )));
    }
    ensure_not_lead(&team, &target)?;
    ensure_level(db, &org.root.id, sub, &target).await?;
    let note = trimmed_note(body.note.as_deref())?;
    let row = match earlier(chat_row(db, &target.id, sub).await?, &target, now)? {
        Earlier::Pending(row) => return Ok((chat_view(&row), Effects::default())),
        Earlier::Fresh => {
            ensure_pending_capacity(db, sub).await?;
            bot_team_chat::ActiveModel {
                bot_id: Set(target.id.clone()),
                user_id: Set(sub.to_string()),
                owner_id: Set(target.owner_id.clone()),
                status: Set(BotChatStatus::Pending.as_str().to_string()),
                conversation_id: Set(None),
                limits: Set(None),
                note: Set(note),
                requested_at: Set(now),
                decided_at: Set(None),
                ended_at: Set(None),
            }
            .insert(db)
            .await?
        }
        Earlier::Renew(row) => {
            ensure_pending_capacity(db, sub).await?;
            let mut renewed: bot_team_chat::ActiveModel = row.into();
            renewed.owner_id = Set(target.owner_id.clone());
            renewed.status = Set(BotChatStatus::Pending.as_str().to_string());
            renewed.note = Set(note);
            renewed.requested_at = Set(now);
            renewed.decided_at = Set(None);
            renewed.ended_at = Set(None);
            renewed.update(db).await?
        }
    };
    let mut effects = Effects::default();
    effects.notice(
        &target.owner_id,
        sub,
        format!("asks to talk with your bot {}", display_name(&target)),
    );
    Ok((chat_view(&row), effects))
}

/// Seats the person, the bot and the watching owner in the chat's conversation.
async fn seat_chat<C: ConnectionTrait>(
    db: &C,
    row: &bot_team_chat::Model,
    owner_id: &str,
    by: &str,
    now: DateTime<FixedOffset>,
    effects: &mut Effects,
) -> Result<(), ApiError> {
    let Some(conversation_id) = row.conversation_id.as_deref() else {
        return Ok(());
    };
    let seats = vec![
        RoomJoin {
            principal: person(&row.user_id),
            limits: None,
        },
        RoomJoin {
            principal: bot_principal(&row.bot_id),
            limits: room_limits(row.limits.as_ref(), true),
        },
        RoomJoin {
            principal: person(owner_id),
            limits: None,
        },
    ];
    seat_members(
        db,
        conversation_id,
        seats,
        &[owner_id.to_string()],
        by,
        now,
        effects,
    )
    .await?;
    announce_watcher(db, conversation_id, &row.bot_id, owner_id, now).await
}

/// Ends the chat: the bot and the owner leave, the person keeps the conversation.
async fn end_row<C: ConnectionTrait>(
    db: &C,
    row: bot_team_chat::Model,
    by: &str,
    now: DateTime<FixedOffset>,
    effects: &mut Effects,
) -> Result<(), ApiError> {
    if let Some(conversation_id) = row.conversation_id.as_deref() {
        let seats = vec![RoomJoin {
            principal: person(&row.user_id),
            limits: None,
        }];
        seat_members(db, conversation_id, seats, &[], by, now, effects).await?;
    }
    let mut ended: bot_team_chat::ActiveModel = row.into();
    ended.status = Set(BotChatStatus::Ended.as_str().to_string());
    ended.ended_at = Set(Some(now));
    ended.update(db).await?;
    Ok(())
}

async fn owner_of<C: ConnectionTrait>(
    db: &C,
    row: &bot_team_chat::Model,
) -> Result<String, sea_orm::DbErr> {
    Ok(live_bot(db, &row.bot_id)
        .await?
        .map_or_else(|| row.owner_id.clone(), |bot| bot.owner_id))
}

/// The pending chat `sub` decides as the bot's owner.
async fn pending_of_owner<C: ConnectionTrait>(
    db: &C,
    sub: &str,
    bot_id: &str,
    user_id: &str,
) -> Result<(bot_team_chat::Model, bot::Model), ApiError> {
    let row = chat_row(db, bot_id, user_id)
        .await?
        .filter(|row| status_of(row) == BotChatStatus::Pending)
        .ok_or_else(|| unknown_chat(bot_id, user_id))?;
    let target = live_bot(db, bot_id)
        .await?
        .filter(|target| target.owner_id == sub)
        .ok_or_else(|| unknown_chat(bot_id, user_id))?;
    Ok((row, target))
}

/// Approves a pending chat with the limits the owner chose and opens its conversation.
pub(crate) async fn accept_chat_in<C: ConnectionTrait>(
    db: &C,
    sub: &str,
    bot_id: &str,
    user_id: &str,
    request: &AcceptChatRequest,
    now: DateTime<FixedOffset>,
) -> Result<(BotChat, Effects), ApiError> {
    let (row, target) = pending_of_owner(db, sub, bot_id, user_id).await?;
    let (team, org) = shared_team(db, bot_id, user_id)
        .await?
        .ok_or_else(|| unknown_chat(bot_id, user_id))?;
    ensure_not_lead(&team, &target)?;
    ensure_level(db, &org.root.id, user_id, &target).await?;
    let limits = request
        .limits
        .as_ref()
        .map(|limits: &BotRoomLimits| {
            validate_limits(limits)?;
            serde_json::to_value(limits).map_err(|error| {
                ApiError::bad_request(format!("Room limits are not valid: {error}"))
            })
        })
        .transpose()?;
    let mut effects = Effects::default();
    let conversation_id = match row.conversation_id.clone() {
        Some(id) => id,
        None => {
            let id = create_id();
            let title = format!(
                "{} · {}",
                display_name(&target),
                person_label(db, user_id).await
            );
            effects.wake(create_room(db, &id, user_id, &title, now).await?);
            id
        }
    };
    let mut accepted: bot_team_chat::ActiveModel = row.into();
    accepted.owner_id = Set(sub.to_string());
    accepted.status = Set(BotChatStatus::Active.as_str().to_string());
    accepted.conversation_id = Set(Some(conversation_id));
    accepted.limits = Set(limits);
    accepted.decided_at = Set(Some(now));
    accepted.ended_at = Set(None);
    let row = accepted.update(db).await?;
    seat_chat(db, &row, sub, sub, now, &mut effects).await?;
    Ok((chat_view(&row), effects))
}

/// Declines a pending chat; the person may ask again after [`CHAT_RETRY_DAYS`].
pub(crate) async fn decline_chat_in<C: ConnectionTrait>(
    db: &C,
    sub: &str,
    bot_id: &str,
    user_id: &str,
    now: DateTime<FixedOffset>,
) -> Result<(), ApiError> {
    let (row, _) = pending_of_owner(db, sub, bot_id, user_id).await?;
    let mut declined: bot_team_chat::ActiveModel = row.into();
    declined.status = Set(BotChatStatus::Declined.as_str().to_string());
    declined.decided_at = Set(Some(now));
    declined.update(db).await?;
    Ok(())
}

/// The person withdraws a pending request or ends an active chat; the bot's owner ends an
/// active one. Anyone else, and any other state, finds nothing.
pub(crate) async fn end_chat_in<C: ConnectionTrait>(
    db: &C,
    sub: &str,
    bot_id: &str,
    user_id: &str,
    now: DateTime<FixedOffset>,
) -> Result<Effects, ApiError> {
    let row = chat_row(db, bot_id, user_id)
        .await?
        .ok_or_else(|| unknown_chat(bot_id, user_id))?;
    let by_person = row.user_id == sub;
    let by_owner = owner_of(db, &row).await? == sub;
    let mut effects = Effects::default();
    match status_of(&row) {
        BotChatStatus::Pending if by_person => {
            bot_team_chat::Entity::delete_by_id((bot_id.to_string(), user_id.to_string()))
                .exec(db)
                .await?;
        }
        BotChatStatus::Active if by_person || by_owner => {
            end_row(db, row, sub, now, &mut effects).await?;
        }
        _ => return Err(unknown_chat(bot_id, user_id)),
    }
    Ok(effects)
}

/// Brings every chat that touches `bot_ids` or `user_ids` in line with the teams: a chat is
/// allowed while its bot is live and active in a team of an org where the person is an active
/// person. Others lose their request, or end; the rest are seated again. The caller holds the
/// locks of the bots and of the chats' conversations ([`lock_chats`]).
pub(crate) async fn sync_chats_of<C: ConnectionTrait>(
    db: &C,
    bot_ids: &[String],
    user_ids: &[String],
    by: &str,
    now: DateTime<FixedOffset>,
    effects: &mut Effects,
) -> Result<(), ApiError> {
    if bot_ids.is_empty() && user_ids.is_empty() {
        return Ok(());
    }
    let touched = Condition::any()
        .add(bot_team_chat::Column::BotId.is_in(bot_ids.iter().cloned()))
        .add(bot_team_chat::Column::UserId.is_in(user_ids.iter().cloned()));
    let rows = bot_team_chat::Entity::find()
        .filter(touched)
        .filter(bot_team_chat::Column::Status.ne(BotChatStatus::Ended.as_str()))
        .all(db)
        .await?;
    for row in rows {
        let allowed = match live_bot(db, &row.bot_id).await? {
            Some(target) => shared_team(db, &row.bot_id, &row.user_id)
                .await?
                .map(|_| target.owner_id),
            None => None,
        };
        match (allowed, status_of(&row)) {
            (Some(owner), BotChatStatus::Active) => {
                seat_chat(db, &row, &owner, by, now, effects).await?;
            }
            (Some(_), _) => {}
            (None, BotChatStatus::Active) => end_row(db, row, by, now, effects).await?,
            (None, _) => {
                bot_team_chat::Entity::delete_by_id((row.bot_id, row.user_id))
                    .exec(db)
                    .await?;
            }
        }
    }
    Ok(())
}

/// Locks the bot and the conversation of its chat with `user_id`, bot first like every team
/// change.
async fn lock_chat(txn: &DatabaseTransaction, bot_id: &str, user_id: &str) -> Result<(), ApiError> {
    lock_bot(txn, bot_id).await?;
    if let Some(conversation_id) = chat_row(txn, bot_id, user_id)
        .await?
        .and_then(|row| row.conversation_id)
    {
        lock_room(txn, &conversation_id).await?;
    }
    Ok(())
}

#[utoipa::path(
    post,
    path = "/bot-teams/chats",
    tag = "Bot teams",
    request_body = ChatRequestBody,
    responses(
        (status = 200, description = "Your request to talk with the bot, waiting for its owner. Asking again while it waits returns the same request.", body = Object),
        (status = 400, description = "The bot is your own, or the note is longer than 280 characters"),
        (status = 401, description = "Not signed in"),
        (status = 403, description = "TEAM_CLASSIFIED: the bot is classified above you in this organization"),
        (status = 404, description = "The bot does not exist or is not in a team of an organization you are in"),
        (status = 409, description = "TEAM_CHAT_LEAD: talk to a team's lead in your thread with the team; TEAM_CHAT_EXISTS: you already talk with it; TEAM_CHAT_DECLINED: its owner declined recently; TEAM_LIMIT: you have 10 open requests")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(name = "POST /bot-teams/chats", skip(state, user, body))]
pub async fn request_chat(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Json(body): Json<ChatRequestBody>,
) -> Result<Json<BotChat>, ApiError> {
    let sub = user.sub()?;
    let (chat, effects) = state
        .transaction(|txn| {
            let (sub, body) = (sub.clone(), body.clone());
            Box::pin(async move {
                lock_chat(txn, &body.bot_id, &sub).await?;
                request_chat_in(txn, &sub, &body, now_millis()).await
            })
        })
        .await?;
    settle(&state, effects).await;
    audit_chat(
        &state,
        &user,
        "bot.team.chat.request",
        &chat.bot_id,
        &sub,
        None,
    )
    .await;
    Ok(Json(chat))
}

#[utoipa::path(
    post,
    path = "/bot-teams/chats/{bot_id}/{user_id}/accept",
    tag = "Bot teams",
    params(
        ("bot_id" = String, Path, description = "Your bot"),
        ("user_id" = String, Path, description = "The person who asked")
    ),
    request_body = AcceptChatRequest,
    responses(
        (status = 200, description = "The chat is open: its conversation holds the person, your bot (which reads along within the limits you set) and you, who only watch.", body = Object),
        (status = 400, description = "The room limits are not valid"),
        (status = 401, description = "Not signed in"),
        (status = 403, description = "TEAM_CLASSIFIED: the bot is classified above the person"),
        (status = 404, description = "No pending request of that person for a bot of yours, or the two no longer share an organization"),
        (status = 409, description = "TEAM_CHAT_LEAD: the bot leads its team now")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(
    name = "POST /bot-teams/chats/{bot_id}/{user_id}/accept",
    skip(state, user, body)
)]
pub async fn accept_chat(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((bot_id, user_id)): Path<(String, String)>,
    Json(body): Json<AcceptChatRequest>,
) -> Result<Json<BotChat>, ApiError> {
    let sub = user.sub()?;
    let (chat, effects) = state
        .transaction(|txn| {
            let (sub, bot_id, user_id, body) =
                (sub.clone(), bot_id.clone(), user_id.clone(), body.clone());
            Box::pin(async move {
                lock_chat(txn, &bot_id, &user_id).await?;
                accept_chat_in(txn, &sub, &bot_id, &user_id, &body, now_millis()).await
            })
        })
        .await?;
    settle(&state, effects).await;
    let limited = serde_json::json!({ "limited": body.limits.is_some() });
    audit_chat(
        &state,
        &user,
        "bot.team.chat.accept",
        &bot_id,
        &user_id,
        Some(limited),
    )
    .await;
    Ok(Json(chat))
}

#[utoipa::path(
    post,
    path = "/bot-teams/chats/{bot_id}/{user_id}/decline",
    tag = "Bot teams",
    params(
        ("bot_id" = String, Path, description = "Your bot"),
        ("user_id" = String, Path, description = "The person who asked")
    ),
    responses(
        (status = 204, description = "The request is declined; the person may ask again after 7 days."),
        (status = 401, description = "Not signed in"),
        (status = 404, description = "No pending request of that person for a bot of yours")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(
    name = "POST /bot-teams/chats/{bot_id}/{user_id}/decline",
    skip(state, user)
)]
pub async fn decline_chat(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((bot_id, user_id)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    let sub = user.sub()?;
    state
        .transaction(|txn| {
            let (sub, bot_id, user_id) = (sub.clone(), bot_id.clone(), user_id.clone());
            Box::pin(async move {
                lock_chat(txn, &bot_id, &user_id).await?;
                decline_chat_in(txn, &sub, &bot_id, &user_id, now_millis()).await
            })
        })
        .await?;
    audit_chat(
        &state,
        &user,
        "bot.team.chat.decline",
        &bot_id,
        &user_id,
        None,
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    delete,
    path = "/bot-teams/chats/{bot_id}/{user_id}",
    tag = "Bot teams",
    params(
        ("bot_id" = String, Path, description = "The bot"),
        ("user_id" = String, Path, description = "The person who asked")
    ),
    responses(
        (status = 204, description = "As the person, your pending request is withdrawn or your chat ends. As the bot's owner, an active chat ends. The bot and its owner leave the conversation; the person keeps it."),
        (status = 401, description = "Not signed in"),
        (status = 404, description = "There is no such request or chat for you to withdraw or end")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(name = "DELETE /bot-teams/chats/{bot_id}/{user_id}", skip(state, user))]
pub async fn end_chat(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((bot_id, user_id)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    let sub = user.sub()?;
    let effects = state
        .transaction(|txn| {
            let (sub, bot_id, user_id) = (sub.clone(), bot_id.clone(), user_id.clone());
            Box::pin(async move {
                lock_chat(txn, &bot_id, &user_id).await?;
                end_chat_in(txn, &sub, &bot_id, &user_id, now_millis()).await
            })
        })
        .await?;
    settle(&state, effects).await;
    let by = serde_json::json!({ "by_person": sub == user_id });
    audit_chat(
        &state,
        &user,
        "bot.team.chat.end",
        &bot_id,
        &user_id,
        Some(by),
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

/// Audits a step of the chat between `user_id` and `bot_id`, with the ids and `extra` codes.
async fn audit_chat(
    state: &AppState,
    user: &AppUser,
    action: &str,
    bot_id: &str,
    user_id: &str,
    extra: Option<serde_json::Value>,
) {
    let mut details = serde_json::json!({ "bot_id": bot_id, "user_id": user_id });
    if let (Some(serde_json::Value::Object(extra)), Some(fields)) = (extra, details.as_object_mut())
    {
        fields.extend(extra);
    }
    audit!(
        state,
        user,
        action,
        "BotTeamChat",
        format!("{bot_id}:{user_id}"),
        details
    );
}

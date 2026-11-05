use super::model::*;
use crate::{
    entity::{conversation, meeting, meeting_message, meeting_participant},
    error::ApiError,
    routes::inbox::{active_membership, blocks::ensure_direct_allowed},
};
use chrono::{DateTime, Duration, FixedOffset};
use flow_like_types::create_id;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, ConnectionTrait, EntityTrait, IntoActiveModel,
    PaginatorTrait, QueryFilter, QueryOrder, QuerySelect,
};

pub(super) const ADMITTED: &str = "ADMITTED";
pub(super) const WAITING: &str = "WAITING";
pub(super) const REMOVED: &str = "REMOVED";
pub(super) const LEFT: &str = "LEFT";
pub(super) const MAX_PARTICIPANTS: u64 = 8;
const MAX_WAITING: u64 = 32;
const MAX_SESSIONS: u64 = 256;
const MAX_MESSAGES: u64 = 2_000;
const SESSION_LEASE_SECONDS: i64 = 90;

pub(super) fn random_secret() -> Result<String, ApiError> {
    let mut bytes = [0_u8; 32];
    getrandom::fill(&mut bytes).map_err(|_| ApiError::internal("Random source unavailable"))?;
    Ok(hex::encode(bytes))
}

pub(super) fn invite_token(key: &[u8; 32], row: &meeting::Model) -> String {
    // Length prefixes and a domain label keep this capability separate from room keys.
    let mut hash = blake3::Hasher::new_keyed(key);
    hash.update(b"flow-like.meeting-invite.v1");
    for value in [&row.id, &row.invite_nonce] {
        hash.update(&(value.len() as u64).to_le_bytes());
        hash.update(value.as_bytes());
    }
    hash.finalize().to_hex().to_string()
}

pub(super) fn valid_invite(key: &[u8; 32], row: &meeting::Model, token: &str) -> bool {
    // blake3::Hash equality compares in constant time, including for malformed tokens.
    row.guests_allowed
        && token.len() == 64
        && blake3::hash(invite_token(key, row).as_bytes()) == blake3::hash(token.as_bytes())
}

pub(super) fn validate_create(
    request: &CreateMeeting,
    now: DateTime<FixedOffset>,
) -> Result<(), ApiError> {
    let title = request.title.trim();
    if title.is_empty() || title.chars().count() > 160 || title.chars().any(char::is_control) {
        return Err(ApiError::bad_request(
            "Meeting title must contain 1 to 160 characters",
        ));
    }
    if request.starts_at < now - Duration::minutes(15)
        || request.ends_at <= now
        || request.ends_at <= request.starts_at
        || request.ends_at - request.starts_at > Duration::hours(24)
    {
        return Err(ApiError::bad_request(
            "Choose a meeting in the future lasting at most 24 hours",
        ));
    }
    Ok(())
}

pub(super) fn open(row: &meeting::Model, now: DateTime<FixedOffset>) -> Result<(), ApiError> {
    if row.ended_at.is_some() || now >= row.ends_at {
        return Err(ApiError::gone("This meeting has ended"));
    }
    if now < row.starts_at - Duration::minutes(15) {
        return Err(ApiError::conflict(
            "Joining opens 15 minutes before the meeting",
        ));
    }
    Ok(())
}

pub(super) async fn find<C: ConnectionTrait>(db: &C, id: &str) -> Result<meeting::Model, ApiError> {
    meeting::Entity::find_by_id(id)
        .one(db)
        .await?
        .ok_or_else(|| ApiError::not_found("Meeting not found"))
}

pub(super) async fn member<C: ConnectionTrait>(
    db: &C,
    row: &meeting::Model,
    sub: &str,
) -> Result<(), ApiError> {
    let membership = active_membership(db, &row.conversation_id, sub).await?;
    if membership.principal_kind != "USER" {
        return Err(ApiError::FORBIDDEN);
    }
    contact_allowed(db, &row.conversation_id, sub).await?;
    Ok(())
}

async fn contact_allowed<C: ConnectionTrait>(
    db: &C,
    conversation_id: &str,
    sub: &str,
) -> Result<(), ApiError> {
    let conversation = conversation::Entity::find_by_id(conversation_id)
        .one(db)
        .await?
        .ok_or_else(|| ApiError::not_found("Conversation not found"))?;
    ensure_direct_allowed(db, &conversation, sub).await
}

pub(super) async fn create_in<C: ConnectionTrait>(
    db: &C,
    sub: &str,
    request: &CreateMeeting,
    now: DateTime<FixedOffset>,
) -> Result<meeting::Model, ApiError> {
    validate_create(request, now)?;
    let membership = active_membership(db, &request.conversation_id, sub).await?;
    if membership.principal_kind != "USER" {
        return Err(ApiError::FORBIDDEN);
    }
    contact_allowed(db, &request.conversation_id, sub).await?;
    let upcoming = meeting::Entity::find()
        .filter(meeting::Column::ConversationId.eq(&request.conversation_id))
        .filter(meeting::Column::EndsAt.gt(now))
        .filter(meeting::Column::EndedAt.is_null())
        .count(db)
        .await?;
    if upcoming >= 50 {
        return Err(ApiError::too_many_requests(
            "This room already has 50 upcoming meetings",
        ));
    }
    Ok(meeting::ActiveModel {
        id: Set(create_id()),
        conversation_id: Set(request.conversation_id.clone()),
        host_id: Set(sub.to_string()),
        title: Set(request.title.trim().to_string()),
        starts_at: Set(request.starts_at),
        ends_at: Set(request.ends_at),
        ended_at: Set(None),
        guests_allowed: Set(request.guests_allowed),
        invite_nonce: Set(random_secret()?),
        room_epoch: Set(0),
        created_at: Set(now),
    }
    .insert(db)
    .await?)
}

pub(super) async fn session_in<C: ConnectionTrait>(
    db: &C,
    row: &meeting::Model,
    token: &str,
) -> Result<meeting_participant::Model, ApiError> {
    let id = token
        .split_once('.')
        .filter(|(_, secret)| secret.len() == 64)
        .filter(|(id, _)| !id.is_empty() && id.len() <= 80)
        .map(|(id, _)| id)
        .ok_or(ApiError::UNAUTHORIZED)?;
    let participant = meeting_participant::Entity::find_by_id(id)
        .one(db)
        .await?
        .filter(|p| p.meeting_id == row.id)
        .ok_or(ApiError::UNAUTHORIZED)?;
    let expected =
        blake3::Hash::from_hex(&participant.session_hash).map_err(|_| ApiError::UNAUTHORIZED)?;
    if expected != blake3::hash(token.as_bytes()) {
        return Err(ApiError::UNAUTHORIZED);
    }
    if let Some(sub) = &participant.user_id {
        member(db, row, sub).await?;
    } else {
        contact_allowed(db, &row.conversation_id, &row.host_id).await?;
    }
    Ok(participant)
}

pub(super) async fn host_in<C: ConnectionTrait>(
    db: &C,
    row: &meeting::Model,
    sub: Option<&str>,
    token: Option<&str>,
) -> Result<(), ApiError> {
    if let Some(sub) = sub {
        if sub != row.host_id {
            return Err(ApiError::forbidden("Only the meeting host can do this"));
        }
        return member(db, row, sub).await;
    }
    if let Some(token) = token {
        let participant = session_in(db, row, token).await?;
        if participant.user_id.as_deref() == Some(&row.host_id) && participant.status == ADMITTED {
            admitted(row, &participant, crate::routes::inbox::now_millis())?;
            return Ok(());
        }
    }
    Err(ApiError::forbidden("Only the meeting host can do this"))
}

pub(super) fn admitted(
    row: &meeting::Model,
    participant: &meeting_participant::Model,
    now: DateTime<FixedOffset>,
) -> Result<(), ApiError> {
    open(row, now)?;
    if participant.status != ADMITTED
        || participant.last_seen_at < now - Duration::seconds(SESSION_LEASE_SECONDS)
    {
        return Err(ApiError::forbidden(
            "Join the meeting and wait for the host to admit you",
        ));
    }
    Ok(())
}

pub(super) async fn reap_in<C: ConnectionTrait>(
    db: &C,
    row: &meeting::Model,
    now: DateTime<FixedOffset>,
) -> Result<(), ApiError> {
    let revoked = meeting_participant::Entity::find()
        .filter(meeting_participant::Column::MeetingId.eq(&row.id))
        .filter(meeting_participant::Column::Status.eq(ADMITTED))
        .filter(
            meeting_participant::Column::LastSeenAt
                .lt(now - Duration::seconds(SESSION_LEASE_SECONDS)),
        )
        .count(db)
        .await?;
    let changed = meeting_participant::Entity::update_many()
        .col_expr(
            meeting_participant::Column::Status,
            sea_orm::sea_query::Expr::value(LEFT),
        )
        .filter(meeting_participant::Column::MeetingId.eq(&row.id))
        .filter(meeting_participant::Column::Status.is_in([WAITING, ADMITTED]))
        .filter(
            meeting_participant::Column::LastSeenAt
                .lt(now - Duration::seconds(SESSION_LEASE_SECONDS)),
        )
        .exec(db)
        .await?;
    if changed.rows_affected > 0 && revoked > 0 {
        rotate_room(db, row).await?;
    }
    Ok(())
}

pub(super) async fn heartbeat_in<C: ConnectionTrait>(
    db: &C,
    row: &meeting::Model,
    participant: meeting_participant::Model,
    now: DateTime<FixedOffset>,
) -> Result<meeting_participant::Model, ApiError> {
    if participant.status != WAITING && participant.status != ADMITTED {
        return Ok(participant);
    }
    if row.ended_at.is_some() || now >= row.ends_at {
        return Ok(participant);
    }
    let stale = participant.last_seen_at < now - Duration::seconds(SESSION_LEASE_SECONDS);
    let revoked = stale && participant.status == ADMITTED;
    let mut updated = participant.into_active_model();
    if stale {
        updated.status = Set(LEFT.to_string());
    }
    updated.last_seen_at = Set(now);
    let updated = updated.update(db).await?;
    if revoked {
        rotate_room(db, row).await?;
    }
    Ok(updated)
}

async fn capacity<C: ConnectionTrait>(
    db: &C,
    row: &meeting::Model,
    is_host: bool,
) -> Result<(), ApiError> {
    let members = meeting_participant::Entity::find()
        .filter(meeting_participant::Column::MeetingId.eq(&row.id))
        .filter(meeting_participant::Column::Status.eq(ADMITTED))
        .all(db)
        .await?;
    // Keep a place for the host even if guests arrive before the host joins the call.
    let host_present = members
        .iter()
        .any(|p| p.user_id.as_deref() == Some(&row.host_id));
    let maximum = MAX_PARTICIPANTS - u64::from(!is_host && !host_present);
    if members.len() as u64 >= maximum {
        return Err(ApiError::conflict("This meeting is full"));
    }
    Ok(())
}

pub(super) async fn rotate_room<C: ConnectionTrait>(
    db: &C,
    row: &meeting::Model,
) -> Result<(), ApiError> {
    let mut updated = row.clone().into_active_model();
    updated.room_epoch = Set(row
        .room_epoch
        .checked_add(1)
        .ok_or_else(|| ApiError::conflict("Meeting has too many membership changes"))?);
    updated.update(db).await?;
    Ok(())
}

pub(super) async fn join_in<C: ConnectionTrait>(
    db: &C,
    row: &meeting::Model,
    sub: Option<&str>,
    name: &str,
    now: DateTime<FixedOffset>,
) -> Result<Joined, ApiError> {
    open(row, now)?;
    if let Some(sub) = sub {
        member(db, row, sub).await?;
    } else {
        contact_allowed(db, &row.conversation_id, &row.host_id).await?;
    }
    if sub.is_none() && !row.guests_allowed {
        return Err(ApiError::forbidden("Guests are disabled for this meeting"));
    }
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 80 || name.chars().any(char::is_control) {
        return Err(ApiError::bad_request(
            "Enter a name with 1 to 80 characters",
        ));
    }
    let existing = if let Some(sub) = sub {
        meeting_participant::Entity::find()
            .filter(meeting_participant::Column::MeetingId.eq(&row.id))
            .filter(meeting_participant::Column::UserId.eq(sub))
            .one(db)
            .await?
    } else {
        None
    };
    if existing.as_ref().is_some_and(|p| p.status == REMOVED) {
        return Err(ApiError::forbidden("You were removed from this meeting"));
    }
    if existing.is_none() {
        let total = meeting_participant::Entity::find()
            .filter(meeting_participant::Column::MeetingId.eq(&row.id))
            .count(db)
            .await?;
        if total >= MAX_SESSIONS {
            return Err(ApiError::too_many_requests("Meeting join limit reached"));
        }
    }
    let was_admitted = existing.as_ref().is_some_and(|p| p.status == ADMITTED);
    let replaced = existing.is_some();
    if sub.is_some() && !was_admitted {
        capacity(db, row, sub == Some(&row.host_id)).await?;
    }
    if sub.is_none() {
        let waiting = meeting_participant::Entity::find()
            .filter(meeting_participant::Column::MeetingId.eq(&row.id))
            .filter(meeting_participant::Column::Status.eq(WAITING))
            .count(db)
            .await?;
        if waiting >= MAX_WAITING {
            return Err(ApiError::too_many_requests("The meeting lobby is full"));
        }
    }
    let id = existing
        .as_ref()
        .map(|p| p.id.clone())
        .unwrap_or_else(create_id);
    let token = format!("{}.{}", id, random_secret()?);
    let hash = blake3::hash(token.as_bytes()).to_hex().to_string();
    if let Some(existing) = existing {
        let mut participant = existing.into_active_model();
        participant.status = Set(ADMITTED.to_string());
        participant.display_name = Set(name.to_string());
        participant.session_hash = Set(hash);
        participant.last_seen_at = Set(now);
        participant.update(db).await?;
    } else {
        meeting_participant::ActiveModel {
            id: Set(id.clone()),
            meeting_id: Set(row.id.clone()),
            user_id: Set(sub.map(str::to_string)),
            display_name: Set(name.to_string()),
            status: Set(if sub.is_some() { ADMITTED } else { WAITING }.to_string()),
            session_hash: Set(hash),
            created_at: Set(now),
            last_seen_at: Set(now),
        }
        .insert(db)
        .await?;
    }
    // Rejoining also changes the room, retiring the old browser's room credential.
    if replaced {
        rotate_room(db, row).await?;
    }
    Ok(Joined {
        session_token: token,
        participant_id: id,
    })
}

pub(super) async fn change_participant_in<C: ConnectionTrait>(
    db: &C,
    row: &meeting::Model,
    id: &str,
    accept: bool,
    now: DateTime<FixedOffset>,
) -> Result<Participant, ApiError> {
    open(row, now)?;
    let target = meeting_participant::Entity::find_by_id(id)
        .one(db)
        .await?
        .filter(|p| p.meeting_id == row.id)
        .ok_or(ApiError::NOT_FOUND)?;
    if target.user_id.as_deref() == Some(&row.host_id) {
        return Err(ApiError::bad_request(
            "The host can leave or end the meeting",
        ));
    }
    if accept {
        if target.status != WAITING {
            return Err(ApiError::conflict("This participant is not waiting"));
        }
        capacity(db, row, false).await?;
    }
    let revoked = !accept && target.status == ADMITTED;
    let mut updated = target.into_active_model();
    updated.status = Set(if accept { ADMITTED } else { REMOVED }.to_string());
    let updated = updated.update(db).await?;
    if revoked {
        rotate_room(db, row).await?;
    }
    Ok(Participant::of(&updated, &row.host_id))
}

pub(super) async fn leave_in<C: ConnectionTrait>(
    db: &C,
    row: &meeting::Model,
    participant: meeting_participant::Model,
) -> Result<(), ApiError> {
    if participant.status == LEFT || participant.status == REMOVED {
        return Ok(());
    }
    let changed = participant.status == ADMITTED;
    let mut participant = participant.into_active_model();
    participant.status = Set(LEFT.to_string());
    participant.update(db).await?;
    if changed {
        rotate_room(db, row).await?;
    }
    Ok(())
}

pub(super) async fn end_in<C: ConnectionTrait>(
    db: &C,
    row: &meeting::Model,
    now: DateTime<FixedOffset>,
) -> Result<(), ApiError> {
    if row.ended_at.is_some() {
        return Ok(());
    }
    let mut updated = row.clone().into_active_model();
    updated.ended_at = Set(Some(now));
    updated.room_epoch = Set(row.room_epoch.saturating_add(1));
    updated.invite_nonce = Set(random_secret()?);
    updated.update(db).await?;
    Ok(())
}

pub(super) async fn send_in<C: ConnectionTrait>(
    db: &C,
    row: &meeting::Model,
    participant: &meeting_participant::Model,
    body: &str,
    now: DateTime<FixedOffset>,
) -> Result<Message, ApiError> {
    admitted(row, participant, now)?;
    let body = body.trim();
    if body.is_empty() || body.len() > 8_000 || body.chars().count() > 2_000 {
        return Err(ApiError::bad_request(
            "Messages must contain 1 to 2000 characters",
        ));
    }
    let count = meeting_message::Entity::find()
        .filter(meeting_message::Column::MeetingId.eq(&row.id))
        .count(db)
        .await?;
    if count >= MAX_MESSAGES {
        return Err(ApiError::too_many_requests(
            "This meeting has reached its chat limit",
        ));
    }
    let recent = meeting_message::Entity::find()
        .filter(meeting_message::Column::MeetingId.eq(&row.id))
        .filter(meeting_message::Column::ParticipantId.eq(&participant.id))
        .filter(meeting_message::Column::CreatedAt.gt(now - Duration::seconds(10)))
        .count(db)
        .await?;
    if recent >= 10 {
        return Err(ApiError::too_many_requests(
            "Please wait before sending more messages",
        ));
    }
    Ok(meeting_message::ActiveModel {
        id: Set(create_id()),
        meeting_id: Set(row.id.clone()),
        participant_id: Set(participant.id.clone()),
        display_name: Set(participant.display_name.clone()),
        body: Set(body.to_string()),
        created_at: Set(now),
    }
    .insert(db)
    .await?
    .into())
}

pub(super) async fn snapshot<C: ConnectionTrait>(
    db: &C,
    row: &meeting::Model,
    me: &meeting_participant::Model,
    now: DateTime<FixedOffset>,
) -> Result<Session, ApiError> {
    let live = row.ended_at.is_none() && now < row.ends_at;
    let mut participants = vec![Participant::of(me, &row.host_id)];
    let mut messages = vec![];
    if live && me.status == ADMITTED {
        participants = meeting_participant::Entity::find()
            .filter(meeting_participant::Column::MeetingId.eq(&row.id))
            .order_by_asc(meeting_participant::Column::CreatedAt)
            .all(db)
            .await?
            .iter()
            .filter(|p| {
                p.status == ADMITTED
                    || (me.user_id.as_deref() == Some(&row.host_id) && p.status == WAITING)
            })
            .map(|p| Participant::of(p, &row.host_id))
            .collect();
        messages = meeting_message::Entity::find()
            .filter(meeting_message::Column::MeetingId.eq(&row.id))
            .order_by_desc(meeting_message::Column::CreatedAt)
            .order_by_desc(meeting_message::Column::Id)
            .limit(100)
            .all(db)
            .await?
            .into_iter()
            .map(Message::from)
            .collect();
        messages.reverse();
    }
    Ok(Session {
        meeting: Meeting::of(row, now),
        me: Participant::of(me, &row.host_id),
        participants,
        messages,
        server_time: now,
        room_epoch: row.room_epoch,
    })
}

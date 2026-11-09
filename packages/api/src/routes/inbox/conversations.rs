use super::{
    Event, INBOX_DIRECT_FIXED, INBOX_MEMBER_LIMIT, MEMBER_ACTIVE, Reader, active_members,
    allocate_seqs,
    delivery::people_to_wake,
    image::sign_images,
    insert_copies,
    invitations::{self, InviteMode, InviteOutcome, Invitee},
    lock_conversation, member_conversation,
    members::{Principal, join_copies, lock_live_bots, requested_members},
    model::{
        Conversation, ConversationList, CreateConversationRequest, ReaderQuery,
        RenameConversationRequest,
    },
    now_millis, reader, requests, stored,
    view::{conversations_for, visible_conversation},
    wake, wake_members,
};
use crate::{
    entity::{bot, conversation, conversation_member},
    error::ApiError,
    middleware::jwt::AppUser,
    routes::bots::model::BotSettings,
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use chrono::{DateTime, FixedOffset};
use flow_like_types::{
    bot::BotCapability,
    bot_effective::BotRoomLimits,
    create_id,
    inbox::{
        ConversationEncryption, ConversationKind, InboxEntryKind, MAX_CONVERSATION_MEMBERS,
        PrincipalKind, RoomEventBody, direct_pair_key,
    },
};
use sea_orm::{
    ActiveModelTrait,
    ActiveValue::Set,
    ColumnTrait, ConnectionTrait, DatabaseTransaction, DbErr, EntityTrait, QueryFilter,
    sea_query::{Expr, OnConflict},
};
use std::collections::BTreeMap;

pub(super) const TITLE_MAX_CHARS: usize = 120;

/// A validated create request: members are deduplicated and never include the creator.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct NewConversation {
    pub kind: ConversationKind,
    pub title: Option<String>,
    pub members: Vec<String>,
    pub encryption: ConversationEncryption,
}

/// A trimmed title, or `None` for an empty one.
pub(super) fn normalize_title(title: Option<String>) -> Result<Option<String>, ApiError> {
    let Some(title) = title.map(|title| title.trim().to_string()) else {
        return Ok(None);
    };
    if title.chars().count() > TITLE_MAX_CHARS {
        return Err(ApiError::bad_request(format!(
            "A conversation title has at most {TITLE_MAX_CHARS} characters; this one has {}",
            title.chars().count()
        )));
    }
    Ok((!title.is_empty()).then_some(title))
}

pub(super) fn validate_new_conversation(
    creator: &str,
    request: CreateConversationRequest,
) -> Result<NewConversation, ApiError> {
    if request.kind == ConversationKind::Direct && !request.bot_limits.is_empty() {
        return Err(ApiError::bad_request(
            "Group memory settings apply only to group conversations",
        ));
    }
    let members = requested_members(creator, request.members);
    let title = normalize_title(request.title)?;
    match request.kind {
        ConversationKind::Direct if members.len() != 1 => {
            return Err(ApiError::bad_request(format!(
                "A direct conversation has exactly one other member; got {}",
                members.len()
            )));
        }
        ConversationKind::Direct if title.is_some() => {
            return Err(ApiError::bad_request(
                "A direct conversation has no title; start a group to name it",
            ));
        }
        ConversationKind::Group if members.is_empty() => {
            return Err(ApiError::bad_request(
                "A group needs at least one member besides you",
            ));
        }
        ConversationKind::Group if members.len() >= MAX_CONVERSATION_MEMBERS => {
            return Err(ApiError::coded(
                StatusCode::CONFLICT,
                INBOX_MEMBER_LIMIT,
                format!(
                    "A conversation has at most {MAX_CONVERSATION_MEMBERS} members; you asked for {} with you",
                    members.len() + 1
                ),
            ));
        }
        _ => {}
    }
    Ok(NewConversation {
        kind: request.kind,
        title,
        members,
        encryption: request.encryption.unwrap_or(ConversationEncryption::Plain),
    })
}

pub(super) fn conversation_row(
    id: &str,
    creator: &str,
    plan: &NewConversation,
    direct_key: Option<String>,
    now: DateTime<FixedOffset>,
) -> conversation::ActiveModel {
    conversation::ActiveModel {
        id: Set(id.to_string()),
        kind: Set(stored(plan.kind.as_str())),
        title: Set(plan.title.clone()),
        encryption: Set(stored(plan.encryption.as_str())),
        direct_key: Set(direct_key),
        created_by: Set(creator.to_string()),
        last_activity_at: Set(now),
        created_at: Set(now),
        updated_at: Set(now),
        import_key: Set(None),
        last_seq: Set(Some(0)),
        image: Set(None),
    }
}

pub(super) fn member_row(
    conversation_id: &str,
    principal: &Principal,
    added_by: Option<&str>,
    now: DateTime<FixedOffset>,
) -> conversation_member::ActiveModel {
    conversation_member::ActiveModel {
        conversation_id: Set(conversation_id.to_string()),
        principal_id: Set(principal.id.clone()),
        principal_kind: Set(stored(principal.kind.as_str())),
        added_by: Set(added_by.map(str::to_string)),
        joined_at: Set(now),
        left_at: Set(None),
        last_read_at: Set(None),
        status: Set(MEMBER_ACTIVE.to_string()),
        limits: Set(None),
        invited_by: Set(None),
        invited_at: Set(None),
        accepted_at: Set(None),
        hidden_at: Set(None),
        cleared_at: Set(None),
        cleared_legacy_at: Set(None),
        cleared_seq: Set(None),
    }
}

/// Starts the direct conversation of the pair unless it exists; concurrent starts keep one.
/// Returns the conversation's id either way.
pub(crate) async fn start_direct<C: ConnectionTrait>(
    db: &C,
    creator: &str,
    other: &Principal,
    encryption: ConversationEncryption,
    now: DateTime<FixedOffset>,
) -> Result<String, ApiError> {
    start_direct_as(db, creator, other, encryption, false, now).await
}

/// [`start_direct`]; as a request the other member is pending until they accept and receives
/// nothing meanwhile.
pub(crate) async fn start_direct_as<C: ConnectionTrait>(
    db: &C,
    creator: &str,
    other: &Principal,
    encryption: ConversationEncryption,
    request: bool,
    now: DateTime<FixedOffset>,
) -> Result<String, ApiError> {
    super::blocks::ensure_allowed(db, creator, &other.id).await?;
    let key = direct_pair_key(creator, &other.id);
    let plan = NewConversation {
        kind: ConversationKind::Direct,
        title: None,
        members: vec![other.id.clone()],
        encryption,
    };
    let id = create_id();
    let inserted = conversation::Entity::insert(conversation_row(
        &id,
        creator,
        &plan,
        Some(key.clone()),
        now,
    ))
    .on_conflict(
        OnConflict::column(conversation::Column::DirectKey)
            .do_nothing()
            .to_owned(),
    )
    .exec_without_returning(db)
    .await?;
    if inserted == 1 {
        let creator = Principal {
            id: creator.to_string(),
            kind: PrincipalKind::User,
        };
        let recipient = if request {
            invitations::pending_row(&id, other, &creator.id, now)
        } else {
            member_row(&id, other, Some(&creator.id), now)
        };
        conversation_member::Entity::insert_many([member_row(&id, &creator, None, now), recipient])
            .exec_without_returning(db)
            .await?;
        return Ok(id);
    }
    direct_conversation_id(db, &key).await?.ok_or_else(|| {
        ApiError::internal("A direct conversation vanished right after a conflicting insert")
    })
}

/// The group the owner and two of their bots talk in, started when missing: one per pair, found
/// by its key, so concurrent starts keep one. The owner reads it; both bots post in it as
/// themselves. Returns the conversation's id either way.
pub(crate) async fn start_exchange<C: ConnectionTrait>(
    db: &C,
    owner_id: &str,
    first_bot: &str,
    second_bot: &str,
    title: Option<String>,
    now: DateTime<FixedOffset>,
) -> Result<String, ApiError> {
    let (low, high) = if first_bot <= second_bot {
        (first_bot, second_bot)
    } else {
        (second_bot, first_bot)
    };
    let key = exchange_key(low, high);
    let plan = NewConversation {
        kind: ConversationKind::Group,
        title: normalize_title(title)?,
        members: vec![low.to_string(), high.to_string()],
        encryption: ConversationEncryption::Plain,
    };
    let id = create_id();
    let inserted = conversation::Entity::insert(conversation_row(
        &id,
        owner_id,
        &plan,
        Some(key.clone()),
        now,
    ))
    .on_conflict(
        OnConflict::column(conversation::Column::DirectKey)
            .do_nothing()
            .to_owned(),
    )
    .exec_without_returning(db)
    .await?;
    if inserted == 0 {
        return direct_conversation_id(db, &key).await?.ok_or_else(|| {
            ApiError::internal("An exchange vanished right after a conflicting insert")
        });
    }
    let owner = Principal {
        id: owner_id.to_string(),
        kind: PrincipalKind::User,
    };
    let bots = [low, high].map(|bot| Principal {
        id: bot.to_string(),
        kind: PrincipalKind::Bot,
    });
    let mut everyone = vec![owner.clone()];
    everyone.extend(bots.iter().cloned());
    conversation_member::Entity::insert_many(everyone.iter().map(|principal| {
        let added_by = (principal.id != owner.id).then_some(owner.id.as_str());
        member_row(&id, principal, added_by, now)
    }))
    .exec_without_returning(db)
    .await?;
    let first_seq = allocate_seqs(db, &id, bots.len()).await?;
    insert_copies(
        db,
        join_copies(&id, &bots, &owner.id, &everyone, now, first_seq),
    )
    .await?;
    Ok(id)
}

/// The key under which a pair of bots has its exchange.
pub(crate) fn exchange_key(low: &str, high: &str) -> String {
    format!("exchange:{low}:{high}")
}

/// How a person's direct conversation with their own bot is encrypted: end to end while the bot
/// has the `encryption` capability.
pub(crate) fn own_bot_encryption(bot: &bot::Model) -> ConversationEncryption {
    if BotSettings::from_row(bot).has(BotCapability::Encryption) {
        ConversationEncryption::E2e
    } else {
        ConversationEncryption::Plain
    }
}

/// Brings the owner's direct conversation with `bot` to [`own_bot_encryption`]. Earlier entries
/// keep their form. Whether it changed.
pub(crate) async fn sync_own_bot_direct<C: ConnectionTrait>(
    db: &C,
    bot: &bot::Model,
) -> Result<bool, DbErr> {
    let encryption = stored(own_bot_encryption(bot).as_str());
    let updated = conversation::Entity::update_many()
        .col_expr(conversation::Column::Encryption, Expr::value(encryption.clone()))
        .col_expr(conversation::Column::UpdatedAt, Expr::value(now_millis()))
        .filter(conversation::Column::DirectKey.eq(direct_pair_key(&bot.owner_id, &bot.id)))
        .filter(conversation::Column::Encryption.ne(encryption))
        .exec(db)
        .await?;
    Ok(updated.rows_affected > 0)
}

/// `id` when it names one of `sub`'s live bots.
async fn own_bot<C: ConnectionTrait>(
    db: &C,
    sub: &str,
    id: &str,
) -> Result<Option<bot::Model>, DbErr> {
    bot::Entity::find_by_id(id)
        .filter(bot::Column::OwnerId.eq(sub))
        .filter(bot::Column::DeletedAt.is_null())
        .one(db)
        .await
}

async fn direct_conversation_id<C: ConnectionTrait>(
    db: &C,
    key: &str,
) -> Result<Option<String>, ApiError> {
    Ok(conversation::Entity::find()
        .filter(conversation::Column::DirectKey.eq(key))
        .one(db)
        .await?
        .map(|row| row.id))
}

/// Starts a group with the creator and `others`, and writes one join entry per added member
/// into every member's inbox, at the group's first positions. Returns the people to wake.
pub(super) async fn start_group<C: ConnectionTrait>(
    db: &C,
    id: &str,
    creator: &str,
    plan: &NewConversation,
    others: &[Principal],
    now: DateTime<FixedOffset>,
) -> Result<Vec<String>, ApiError> {
    conversation_row(id, creator, plan, None, now)
        .insert(db)
        .await?;
    let creator = Principal {
        id: creator.to_string(),
        kind: PrincipalKind::User,
    };
    let mut everyone = vec![creator.clone()];
    everyone.extend(others.iter().cloned());
    conversation_member::Entity::insert_many(everyone.iter().map(|principal| {
        let added_by = (principal.id != creator.id).then_some(creator.id.as_str());
        member_row(id, principal, added_by, now)
    }))
    .exec_without_returning(db)
    .await?;
    if others.is_empty() {
        return Ok(people_to_wake(db, &everyone).await?);
    }
    let first_seq = allocate_seqs(db, id, others.len()).await?;
    insert_copies(
        db,
        join_copies(id, others, &creator.id, &everyone, now, first_seq),
    )
    .await?;
    Ok(people_to_wake(db, &everyone).await?)
}

#[utoipa::path(
    get,
    path = "/inbox/conversations",
    tag = "Inbox",
    params(ReaderQuery),
    responses(
        (status = 200, description = "Your conversations, newest activity first (at most 200), each with its members, your unread count and the newest message. Reading as one of your bots lists every conversation it is in, its exchanges with other bots included.", body = ConversationList),
        (status = 401, description = "Not signed in"),
        (status = 403, description = "Only people read inboxes, not API keys or running flows"),
        (status = 404, description = "`as` is not one of your bots")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(name = "GET /inbox/conversations", skip(state, user))]
pub async fn list_conversations(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Query(query): Query<ReaderQuery>,
) -> Result<Json<ConversationList>, ApiError> {
    let sub = user.sub()?;
    let reader = reader(&state.db, &sub, query.as_bot.as_deref()).await?;
    let mut conversations = conversations_for(&state.db, &reader, None).await?;
    sign_images(&state, &mut conversations).await;
    Ok(Json(ConversationList { conversations }))
}

#[utoipa::path(
    get,
    path = "/inbox/conversations/{conversation_id}",
    tag = "Inbox",
    params(
        ("conversation_id" = String, Path, description = "The conversation"),
        ReaderQuery
    ),
    responses(
        (status = 200, description = "The conversation with its members, your unread count and the newest message", body = Conversation),
        (status = 401, description = "Not signed in"),
        (status = 403, description = "Only people read inboxes, not API keys or running flows"),
        (status = 404, description = "You (or, reading as a bot, the bot) are not in this conversation, or `as` is not one of your bots")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(name = "GET /inbox/conversations/{conversation_id}", skip(state, user))]
pub async fn get_conversation(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(conversation_id): Path<String>,
    Query(query): Query<ReaderQuery>,
) -> Result<Json<Conversation>, ApiError> {
    let reader =
        super::acting_reader(&state.db, &user, query.as_bot.as_deref(), &conversation_id).await?;
    Ok(Json(
        visible_conversation(&state, &reader, &conversation_id).await?,
    ))
}

/// The pair's direct conversation: the existing one, which writing answers if you hold a
/// request in it, or a new one once the other member passes the member rules. Whoever isn't
/// one of your own bots receives it as a request. A conversation with your own bot is encrypted
/// as the bot is ([`own_bot_encryption`]), whatever the request asks. Returns the id.
async fn open_direct(
    state: &AppState,
    sub: &str,
    plan: &NewConversation,
) -> Result<String, ApiError> {
    let key = direct_pair_key(sub, &plan.members[0]);
    let own = own_bot(&state.db, sub, &plan.members[0]).await?;
    if let Some(id) = direct_conversation_id(&state.db, &key).await? {
        let people = state
            .transaction(|txn| {
                let (id, sub, own) = (id.clone(), sub.to_string(), own.clone());
                Box::pin(async move {
                    lock_conversation(txn, &id).await?;
                    if let Some(bot) = &own {
                        sync_own_bot_direct(txn, bot).await?;
                    }
                    let mut people =
                        requests::answer_by_writing(txn, &id, &sub, now_millis()).await?;
                    super::deletion::reopen(txn, &id, &sub).await?;
                    people.push(sub);
                    Ok::<_, ApiError>(people)
                })
            })
            .await?;
        wake(state, people).await;
        return Ok(id);
    }
    let encryption = own.as_ref().map_or(plan.encryption, own_bot_encryption);
    let direct = requests::plan_direct(&state.db, sub, &plan.members[0], encryption).await?;
    let id = state
        .transaction(|txn| {
            let (sub, direct) = (sub.to_string(), direct.clone());
            Box::pin(async move {
                if !direct.request {
                    lock_live_bots(txn, &sub, std::slice::from_ref(&direct.other)).await?;
                }
                let id = start_direct_as(
                    txn,
                    &sub,
                    &direct.other,
                    encryption,
                    direct.request,
                    now_millis(),
                )
                .await?;
                lock_conversation(txn, &id).await?;
                super::deletion::reopen(txn, &id, &sub).await?;
                Ok::<_, ApiError>(id)
            })
        })
        .await?;
    wake(state, [sub.to_string()]).await;
    if direct.request {
        wake(
            state,
            people_to_wake(&state.db, std::slice::from_ref(&direct.other)).await?,
        )
        .await;
    }
    Ok(id)
}

/// Creates the group and applies its initial invitations and limits in one transaction.
/// The caller locks its own bots before calling this helper.
pub(super) async fn create_group_in<C: ConnectionTrait>(
    txn: &C,
    id: &str,
    sub: &str,
    plan: &NewConversation,
    invitees: &[Invitee],
    limits: &BTreeMap<String, BotRoomLimits>,
    now: DateTime<FixedOffset>,
) -> Result<InviteOutcome, ApiError> {
    let wake = start_group(txn, id, sub, plan, &[], now).await?;
    let mut outcome = invitations::invite_in(txn, id, sub, invitees, limits, now).await?;
    outcome.wake.extend(wake);
    Ok(outcome)
}

async fn open_group(
    state: &AppState,
    sub: &str,
    plan: &NewConversation,
    limits: BTreeMap<String, BotRoomLimits>,
) -> Result<String, ApiError> {
    let invitees = invitations::classify(&state.db, sub, &plan.members).await?;
    let others: Vec<Principal> = invitees
        .iter()
        .filter(|invitee| invitee.mode == InviteMode::AddNow)
        .map(|invitee| invitee.principal.clone())
        .collect();
    let id = create_id();
    let outcome = state
        .transaction(|txn| {
            let (id, sub, plan, others, invitees, limits) = (
                id.clone(),
                sub.to_string(),
                plan.clone(),
                others.clone(),
                invitees.clone(),
                limits.clone(),
            );
            Box::pin(async move {
                lock_live_bots(txn, &sub, &others).await?;
                create_group_in(txn, &id, &sub, &plan, &invitees, &limits, now_millis()).await
            })
        })
        .await?;
    wake(state, outcome.wake).await;
    invitations::notify_invited(state, sub, &id, &outcome.invited).await;
    Ok(id)
}

#[utoipa::path(
    post,
    path = "/inbox/conversations",
    tag = "Inbox",
    request_body = CreateConversationRequest,
    responses(
        (status = 200, description = "The new conversation. A direct conversation you already have with this member is returned instead of a new one, whatever encryption you asked for.", body = Conversation),
        (status = 400, description = "A direct conversation needs exactly one other member and no title, a group at least one; or the title is too long"),
        (status = 401, description = "Not signed in"),
        (status = 403, description = "INBOX_MEMBER_NOT_ALLOWED: an id is neither a person on this hub nor a live bot; the message names the first id refused"),
        (status = 409, description = "INBOX_MEMBER_LIMIT: a conversation has at most 50 members")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(name = "POST /inbox/conversations", skip(state, user, body))]
pub async fn create_conversation(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Json(body): Json<CreateConversationRequest>,
) -> Result<Json<Conversation>, ApiError> {
    let sub = user.sub()?;
    let limits = body.bot_limits.clone();
    let plan = validate_new_conversation(&sub, body)?;
    let id = match plan.kind {
        ConversationKind::Direct => open_direct(&state, &sub, &plan).await?,
        ConversationKind::Group => open_group(&state, &sub, &plan, limits).await?,
    };
    Ok(Json(
        visible_conversation(&state, &Reader::own(&sub), &id).await?,
    ))
}

#[utoipa::path(
    patch,
    path = "/inbox/conversations/{conversation_id}",
    tag = "Inbox",
    params(("conversation_id" = String, Path, description = "The group")),
    request_body = RenameConversationRequest,
    responses(
        (status = 200, description = "The group with its new title", body = Conversation),
        (status = 400, description = "The title has more than 120 characters"),
        (status = 401, description = "Not signed in"),
        (status = 403, description = "Only people rename conversations, not API keys or running flows"),
        (status = 404, description = "You are not in this conversation"),
        (status = 409, description = "INBOX_DIRECT_FIXED: a direct conversation has no title")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(
    name = "PATCH /inbox/conversations/{conversation_id}",
    skip(state, user, body)
)]
pub async fn rename_conversation(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(conversation_id): Path<String>,
    Json(body): Json<RenameConversationRequest>,
) -> Result<Json<Conversation>, ApiError> {
    let sub = user.sub()?;
    let title = normalize_title(body.title)?;
    let renamed = state
        .transaction(|txn| {
            let (id, sub, title) = (conversation_id.clone(), sub.clone(), title.clone());
            Box::pin(async move { rename_in(txn, &id, &sub, title, now_millis()).await })
        })
        .await?;
    if renamed {
        wake_members(&state, &conversation_id).await;
    }
    Ok(Json(
        visible_conversation(&state, &Reader::own(&sub), &conversation_id).await?,
    ))
}

/// Renames the group and records it in every member's history; false when the name is unchanged.
pub(super) async fn rename_in(
    txn: &DatabaseTransaction,
    conversation_id: &str,
    by: &str,
    title: Option<String>,
    now: DateTime<FixedOffset>,
) -> Result<bool, ApiError> {
    lock_conversation(txn, conversation_id).await?;
    let current = member_group(txn, conversation_id, by).await?;
    if current.title == title {
        return Ok(false);
    }
    conversation::Entity::update_many()
        .col_expr(conversation::Column::Title, Expr::value(title.clone()))
        .col_expr(conversation::Column::UpdatedAt, Expr::value(now))
        .filter(conversation::Column::Id.eq(conversation_id))
        .exec(txn)
        .await?;
    record_room_change(txn, conversation_id, &RoomEventBody::title(title, by), now).await?;
    Ok(true)
}

/// Writes a `room` entry into the inbox of every active member at the group's next position.
/// The caller holds the conversation's lock.
pub(super) async fn record_room_change<C: ConnectionTrait>(
    txn: &C,
    conversation_id: &str,
    change: &RoomEventBody,
    now: DateTime<FixedOffset>,
) -> Result<(), ApiError> {
    let members = active_members(txn, &[conversation_id.to_string()]).await?;
    let seq = allocate_seqs(txn, conversation_id, 1).await?;
    let message_id = format!("room:{}", create_id());
    let event = Event {
        conversation_id,
        message_id: &message_id,
        sender_id: &change.by,
        kind: InboxEntryKind::Room,
        created_at: now,
        seq,
    };
    let envelope = change.envelope();
    insert_copies(
        txn,
        members
            .iter()
            .map(|member| event.copy_for(&member.principal_id, envelope.clone()))
            .collect(),
    )
    .await
}

/// The group `principal_id` is an active member of; a direct conversation is fixed.
pub(super) async fn member_group<C: ConnectionTrait>(
    db: &C,
    conversation_id: &str,
    principal_id: &str,
) -> Result<conversation::Model, ApiError> {
    let current = member_conversation(db, conversation_id, principal_id).await?;
    if ConversationKind::parse(&current.kind) != Some(ConversationKind::Group) {
        return Err(direct_fixed());
    }
    Ok(current)
}

pub(super) fn direct_fixed() -> ApiError {
    ApiError::coded(
        StatusCode::CONFLICT,
        INBOX_DIRECT_FIXED,
        "A direct conversation has no title or picture and its members don't change; start a group instead",
    )
}

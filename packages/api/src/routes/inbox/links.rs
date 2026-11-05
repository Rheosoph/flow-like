//! Invite links for groups. Any active member makes one; whoever opens it signed in joins the
//! group at once while the link is live: not expired, not used up, and its creator still an
//! active member. Encrypted groups take no links yet, since someone joining may have no key to
//! seal messages to. A token is a credential: logs and audit records carry the link id only.

use super::{
    INBOX_LINK_LIMIT, INBOX_LINK_NOT_ALLOWED, INBOX_LINK_NOT_FOUND, INBOX_LINK_USED_UP, Reader,
    active_members, allocate_seqs,
    delivery::people_to_wake,
    insert_copies, lock_conversation, member_conversation,
    members::{Principal, ensure_room, group_members, join_copies, upsert_members},
    model::Conversation,
    now_millis,
    view::visible_conversation,
    wake,
};
use crate::{
    audit,
    entity::{conversation, conversation_invite_link},
    error::ApiError,
    middleware::jwt::AppUser,
    routes::user::ensure_user_exists,
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
    http::StatusCode,
};
use chrono::{DateTime, Duration, FixedOffset};
use flow_like_types::{
    base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD},
    create_id,
    inbox::{ConversationEncryption, ConversationKind, PrincipalKind},
    rand::{TryRngCore, rngs::OsRng},
};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, Condition, ConnectionTrait, DbErr, EntityTrait, PaginatorTrait,
    QueryFilter, QueryOrder, QuerySelect,
    sea_query::{Expr, ExprTrait},
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Links one group holds, live or not; revoking frees a slot.
pub(crate) const MAX_LINKS: u64 = 20;
const NAME_MAX_CHARS: usize = 80;
const MAX_LINK_USES: i64 = 10_000;
const MAX_EXPIRY_HOURS: i64 = 24 * 365;
const TOKEN_BYTES: usize = 32;
const TOKEN_MAX_CHARS: usize = 128;

#[derive(Debug, Clone, Default, Deserialize, ToSchema)]
#[serde(default)]
pub struct CreateConversationLinkRequest {
    /// A label members see in the group's link list; at most 80 characters.
    pub name: Option<String>,
    /// How many people may join with it, at most 10,000; omitted, 0 or less is no limit.
    pub max_uses: Option<i64>,
    /// Hours until it stops working, at most a year; omitted, 0 or less never expires.
    pub expires_in_hours: Option<i64>,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct JoinConversationLinkRequest {
    /// The `room` value of the invite link.
    pub token: String,
}

/// An invite link to a group, as its members see it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct ConversationLink {
    pub id: String,
    pub conversation_id: String,
    /// The secret part of the link: share `{api}/join?room={token}`.
    pub token: String,
    pub name: Option<String>,
    /// The member who made it; the link stops working once they leave.
    pub created_by: String,
    /// -1 is unlimited.
    pub max_uses: i64,
    pub count_joined: i64,
    pub expires_at: Option<DateTime<FixedOffset>>,
    pub created_at: DateTime<FixedOffset>,
}

impl From<conversation_invite_link::Model> for ConversationLink {
    fn from(row: conversation_invite_link::Model) -> Self {
        Self {
            id: row.id,
            conversation_id: row.conversation_id,
            token: row.token,
            name: row.name,
            created_by: row.created_by,
            max_uses: row.max_uses,
            count_joined: row.count_joined,
            expires_at: row.expires_at,
            created_at: row.created_at,
        }
    }
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ConversationLinkList {
    /// Newest first, at most 20.
    pub links: Vec<ConversationLink>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct JoinedConversation {
    pub conversation: Conversation,
    /// False when you were already a member; nothing changed then.
    pub joined: bool,
}

/// A validated [`CreateConversationLinkRequest`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NewLink {
    pub name: Option<String>,
    pub max_uses: i64,
    pub expires_in: Option<Duration>,
}

pub(crate) fn validate_link(request: CreateConversationLinkRequest) -> Result<NewLink, ApiError> {
    let name = request
        .name
        .map(|name| name.trim().to_string())
        .filter(|name| !name.is_empty());
    if let Some(name) = &name
        && name.chars().count() > NAME_MAX_CHARS
    {
        return Err(ApiError::bad_request(format!(
            "An invite link's name has at most {NAME_MAX_CHARS} characters; got {}",
            name.chars().count()
        )));
    }
    let max_uses = match request.max_uses {
        Some(uses) if uses > MAX_LINK_USES => {
            return Err(ApiError::bad_request(format!(
                "An invite link allows at most {MAX_LINK_USES} uses; got {uses}"
            )));
        }
        Some(uses) if uses > 0 => uses,
        _ => -1,
    };
    let expires_in = match request.expires_in_hours {
        Some(hours) if hours > MAX_EXPIRY_HOURS => {
            return Err(ApiError::bad_request(format!(
                "An invite link expires within {MAX_EXPIRY_HOURS} hours; got {hours}"
            )));
        }
        Some(hours) if hours > 0 => Some(Duration::hours(hours)),
        _ => None,
    };
    Ok(NewLink {
        name,
        max_uses,
        expires_in,
    })
}

fn new_token() -> Result<String, ApiError> {
    let mut bytes = [0u8; TOKEN_BYTES];
    OsRng.try_fill_bytes(&mut bytes).map_err(|error| {
        ApiError::internal(format!(
            "Failed to generate the token of an invite link: {error}"
        ))
    })?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}

/// A token can only be one this module minted: URL-safe base64, bounded.
pub(crate) fn is_token(token: &str) -> bool {
    (1..=TOKEN_MAX_CHARS).contains(&token.len())
        && token
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

fn link_not_found() -> ApiError {
    ApiError::coded(
        StatusCode::NOT_FOUND,
        INBOX_LINK_NOT_FOUND,
        "This invite link is invalid, expired or was revoked",
    )
}

fn encrypted(conversation: &conversation::Model) -> bool {
    ConversationEncryption::parse(&conversation.encryption) == Some(ConversationEncryption::E2e)
}

fn joinable(conversation: &conversation::Model) -> bool {
    ConversationKind::parse(&conversation.kind) == Some(ConversationKind::Group)
        && !encrypted(conversation)
}

fn expired(link: &conversation_invite_link::Model, now: DateTime<FixedOffset>) -> bool {
    link.expires_at.is_some_and(|expires_at| expires_at <= now)
}

fn used_up(link: &conversation_invite_link::Model) -> bool {
    link.max_uses > 0 && link.count_joined >= link.max_uses
}

async fn link_by_token<C: ConnectionTrait>(
    db: &C,
    token: &str,
) -> Result<Option<conversation_invite_link::Model>, DbErr> {
    if !is_token(token) {
        return Ok(None);
    }
    conversation_invite_link::Entity::find()
        .filter(conversation_invite_link::Column::Token.eq(token))
        .one(db)
        .await
}

/// A link someone could join with right now, and the group it leads to.
#[derive(Debug, Clone)]
pub(crate) struct LiveLink {
    pub link: conversation_invite_link::Model,
    pub conversation: conversation::Model,
    pub members: usize,
}

/// The link behind `token` when it would let a newcomer in at `now`; anything else reads as
/// missing, so a preview never tells a revoked link from a mistyped one.
pub(crate) async fn live_link<C: ConnectionTrait>(
    db: &C,
    token: &str,
    now: DateTime<FixedOffset>,
) -> Result<Option<LiveLink>, DbErr> {
    let Some(link) = link_by_token(db, token).await? else {
        return Ok(None);
    };
    if expired(&link, now) || used_up(&link) {
        return Ok(None);
    }
    let Some(conversation) = conversation::Entity::find_by_id(&link.conversation_id)
        .one(db)
        .await?
    else {
        return Ok(None);
    };
    if !joinable(&conversation) {
        return Ok(None);
    }
    let members = active_members(db, std::slice::from_ref(&conversation.id)).await?;
    if !members
        .iter()
        .any(|member| member.principal_id == link.created_by)
    {
        return Ok(None);
    }
    Ok(Some(LiveLink {
        link,
        conversation,
        members: members.len(),
    }))
}

/// Makes a link to the group `creator` is an active member of.
pub(crate) async fn create_link_in<C: ConnectionTrait>(
    txn: &C,
    conversation_id: &str,
    creator: &str,
    link: &NewLink,
    now: DateTime<FixedOffset>,
) -> Result<conversation_invite_link::Model, ApiError> {
    let (conversation, _) = group_members(txn, conversation_id, creator).await?;
    if encrypted(&conversation) {
        return Err(ApiError::coded(
            StatusCode::CONFLICT,
            INBOX_LINK_NOT_ALLOWED,
            "Encrypted groups can't have invite links yet: someone joining may have no key to seal messages to",
        ));
    }
    let held = conversation_invite_link::Entity::find()
        .filter(conversation_invite_link::Column::ConversationId.eq(conversation_id))
        .count(txn)
        .await?;
    if held >= MAX_LINKS {
        return Err(ApiError::coded(
            StatusCode::CONFLICT,
            INBOX_LINK_LIMIT,
            format!(
                "A group holds at most {MAX_LINKS} invite links; revoke one of the {held} in '{conversation_id}' first"
            ),
        ));
    }
    let row = conversation_invite_link::Model {
        id: create_id(),
        conversation_id: conversation_id.to_string(),
        token: new_token()?,
        name: link.name.clone(),
        created_by: creator.to_string(),
        max_uses: link.max_uses,
        count_joined: 0,
        expires_at: link.expires_in.map(|expires_in| now + expires_in),
        created_at: now,
        updated_at: now,
    };
    Ok(conversation_invite_link::ActiveModel::from(row)
        .reset_all()
        .insert(txn)
        .await?)
}

/// The group's links, newest first, for an active member.
pub(crate) async fn links_in<C: ConnectionTrait>(
    db: &C,
    conversation_id: &str,
    member: &str,
) -> Result<Vec<conversation_invite_link::Model>, ApiError> {
    member_conversation(db, conversation_id, member).await?;
    Ok(conversation_invite_link::Entity::find()
        .filter(conversation_invite_link::Column::ConversationId.eq(conversation_id))
        .order_by_desc(conversation_invite_link::Column::CreatedAt)
        .order_by_desc(conversation_invite_link::Column::Id)
        .limit(MAX_LINKS)
        .all(db)
        .await?)
}

/// Deletes a link if `caller` made it or started the group.
pub(crate) async fn revoke_link_in<C: ConnectionTrait>(
    txn: &C,
    conversation_id: &str,
    link_id: &str,
    caller: &str,
) -> Result<(), ApiError> {
    let conversation = member_conversation(txn, conversation_id, caller).await?;
    let link = conversation_invite_link::Entity::find_by_id(link_id)
        .filter(conversation_invite_link::Column::ConversationId.eq(conversation_id))
        .one(txn)
        .await?
        .ok_or_else(|| {
            ApiError::not_found(format!(
                "No invite link '{link_id}' in conversation '{conversation_id}'"
            ))
        })?;
    if link.created_by != caller && conversation.created_by != caller {
        return Err(ApiError::coded(
            StatusCode::FORBIDDEN,
            INBOX_LINK_NOT_ALLOWED,
            "You can revoke invite links you made, or any link in a group you started",
        ));
    }
    conversation_invite_link::Entity::delete_by_id(link_id)
        .exec(txn)
        .await?;
    Ok(())
}

/// What redeeming a link changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LinkJoin {
    pub conversation_id: String,
    pub link_id: String,
    pub joined: bool,
    pub wake: Vec<String>,
}

/// Counts one use unless the link is used up; the guarded update keeps concurrent redeems of a
/// limited link exact.
async fn claim_use<C: ConnectionTrait>(
    txn: &C,
    link_id: &str,
    now: DateTime<FixedOffset>,
) -> Result<bool, DbErr> {
    let claimed = conversation_invite_link::Entity::update_many()
        .col_expr(
            conversation_invite_link::Column::CountJoined,
            Expr::col(conversation_invite_link::Column::CountJoined).add(1),
        )
        .col_expr(
            conversation_invite_link::Column::UpdatedAt,
            Expr::value(now),
        )
        .filter(conversation_invite_link::Column::Id.eq(link_id))
        .filter(
            Condition::any()
                .add(conversation_invite_link::Column::MaxUses.lte(0))
                .add(
                    Expr::col(conversation_invite_link::Column::CountJoined)
                        .lt(Expr::col(conversation_invite_link::Column::MaxUses)),
                ),
        )
        .exec(txn)
        .await?;
    Ok(claimed.rows_affected == 1)
}

/// Adds `joiner` to the link's group at once. A member already in it gets the group back
/// unchanged and uses nothing, so a retried join succeeds. The newcomer counts as added by the
/// link's creator; everyone, the newcomer included, gets a join entry.
pub(crate) async fn join_link_in<C: ConnectionTrait>(
    txn: &C,
    token: &str,
    joiner: &str,
    now: DateTime<FixedOffset>,
) -> Result<LinkJoin, ApiError> {
    let link = link_by_token(txn, token)
        .await?
        .ok_or_else(link_not_found)?;
    let conversation = conversation::Entity::find_by_id(&link.conversation_id)
        .one(txn)
        .await?
        .filter(joinable)
        .ok_or_else(link_not_found)?;
    let current = active_members(txn, std::slice::from_ref(&conversation.id)).await?;
    let unchanged = LinkJoin {
        conversation_id: conversation.id.clone(),
        link_id: link.id.clone(),
        joined: false,
        wake: Vec::new(),
    };
    if current.iter().any(|member| member.principal_id == joiner) {
        return Ok(unchanged);
    }
    let creator_present = current
        .iter()
        .any(|member| member.principal_id == link.created_by);
    if expired(&link, now) || !creator_present {
        return Err(link_not_found());
    }
    ensure_room(&conversation.id, current.len(), 1)?;
    if !claim_use(txn, &link.id, now).await? {
        return Err(ApiError::coded(
            StatusCode::FORBIDDEN,
            INBOX_LINK_USED_UP,
            "This invite link has reached its usage limit",
        ));
    }
    let joining = Principal {
        id: joiner.to_string(),
        kind: PrincipalKind::User,
    };
    upsert_members(
        txn,
        &conversation.id,
        std::slice::from_ref(&joining),
        &link.created_by,
        now,
    )
    .await?;
    let mut everyone: Vec<Principal> = current.iter().map(Principal::from).collect();
    everyone.push(joining.clone());
    let first_seq = allocate_seqs(txn, &conversation.id, 1).await?;
    insert_copies(
        txn,
        join_copies(
            &conversation.id,
            std::slice::from_ref(&joining),
            joiner,
            &everyone,
            now,
            first_seq,
        ),
    )
    .await?;
    Ok(LinkJoin {
        joined: true,
        wake: people_to_wake(txn, &everyone).await?,
        ..unchanged
    })
}

#[utoipa::path(
    post,
    path = "/inbox/conversations/{conversation_id}/links",
    tag = "Inbox",
    params(("conversation_id" = String, Path, description = "The group")),
    request_body = CreateConversationLinkRequest,
    responses(
        (status = 200, description = "A new invite link. Anyone signed in who opens it joins the group at once, until it expires, reaches its usage limit or is revoked, and only while you stay in the group.", body = ConversationLink),
        (status = 400, description = "The name, usage limit or expiry is out of range"),
        (status = 401, description = "Not signed in"),
        (status = 404, description = "You are not in this conversation"),
        (status = 409, description = "INBOX_LINK_NOT_ALLOWED: encrypted groups can't have invite links yet; INBOX_LINK_LIMIT: a group holds at most 20 links; INBOX_DIRECT_FIXED: a direct conversation's members don't change")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(
    name = "POST /inbox/conversations/{conversation_id}/links",
    skip(state, user, body)
)]
pub async fn create_link(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(conversation_id): Path<String>,
    Json(body): Json<CreateConversationLinkRequest>,
) -> Result<Json<ConversationLink>, ApiError> {
    let sub = user.sub()?;
    let link = validate_link(body)?;
    let row = state
        .transaction(|txn| {
            let (id, sub, link) = (conversation_id.clone(), sub.clone(), link.clone());
            Box::pin(async move {
                lock_conversation(txn, &id).await?;
                create_link_in(txn, &id, &sub, &link, now_millis()).await
            })
        })
        .await?;
    audit!(
        state,
        user,
        "inbox.link.create",
        "ConversationInviteLink",
        row.id,
        serde_json::json!({ "conversation_id": row.conversation_id, "max_uses": row.max_uses })
    );
    Ok(Json(ConversationLink::from(row)))
}

#[utoipa::path(
    get,
    path = "/inbox/conversations/{conversation_id}/links",
    tag = "Inbox",
    params(("conversation_id" = String, Path, description = "The group")),
    responses(
        (status = 200, description = "The group's invite links, newest first, with how often each was used. Expired and used-up links stay listed until someone revokes them.", body = ConversationLinkList),
        (status = 401, description = "Not signed in"),
        (status = 404, description = "You are not in this conversation")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(
    name = "GET /inbox/conversations/{conversation_id}/links",
    skip(state, user)
)]
pub async fn list_links(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(conversation_id): Path<String>,
) -> Result<Json<ConversationLinkList>, ApiError> {
    let sub = user.sub()?;
    let links = links_in(&state.db, &conversation_id, &sub).await?;
    Ok(Json(ConversationLinkList {
        links: links.into_iter().map(ConversationLink::from).collect(),
    }))
}

#[utoipa::path(
    delete,
    path = "/inbox/conversations/{conversation_id}/links/{link_id}",
    tag = "Inbox",
    params(
        ("conversation_id" = String, Path, description = "The group"),
        ("link_id" = String, Path, description = "The invite link")
    ),
    responses(
        (status = 204, description = "The link stops working at once; people who joined with it stay"),
        (status = 401, description = "Not signed in"),
        (status = 403, description = "INBOX_LINK_NOT_ALLOWED: revoke links you made, or any link in a group you started"),
        (status = 404, description = "You are not in this conversation, or it has no such link")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(
    name = "DELETE /inbox/conversations/{conversation_id}/links/{link_id}",
    skip(state, user)
)]
pub async fn revoke_link(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((conversation_id, link_id)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    let sub = user.sub()?;
    state
        .transaction(|txn| {
            let (id, link_id, sub) = (conversation_id.clone(), link_id.clone(), sub.clone());
            Box::pin(async move {
                lock_conversation(txn, &id).await?;
                revoke_link_in(txn, &id, &link_id, &sub).await
            })
        })
        .await?;
    audit!(
        state,
        user,
        "inbox.link.revoke",
        "ConversationInviteLink",
        link_id,
        serde_json::json!({ "conversation_id": conversation_id })
    );
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    post,
    path = "/inbox/links/join",
    tag = "Inbox",
    request_body = JoinConversationLinkRequest,
    responses(
        (status = 200, description = "You are in the group and everyone in it gets a join entry. Opening a link to a group you are already in changes nothing and returns it too.", body = JoinedConversation),
        (status = 401, description = "Not signed in"),
        (status = 403, description = "INBOX_LINK_USED_UP: the link reached its usage limit"),
        (status = 404, description = "INBOX_LINK_NOT_FOUND: the link is invalid, expired or revoked, or the person who made it left the group"),
        (status = 409, description = "INBOX_MEMBER_LIMIT: the group is full")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(name = "POST /inbox/links/join", skip(state, user, body))]
pub async fn join_link(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Json(body): Json<JoinConversationLinkRequest>,
) -> Result<Json<JoinedConversation>, ApiError> {
    let sub = user.sub()?;
    let token = body.token.trim().to_string();
    let conversation_id = link_by_token(&state.db, &token)
        .await?
        .map(|link| link.conversation_id)
        .ok_or_else(link_not_found)?;
    ensure_user_exists(&state, &sub).await?;
    let outcome = state
        .transaction(|txn| {
            let (id, token, sub) = (conversation_id.clone(), token.clone(), sub.clone());
            Box::pin(async move {
                lock_conversation(txn, &id).await?;
                join_link_in(txn, &token, &sub, now_millis()).await
            })
        })
        .await?;
    wake(&state, outcome.wake.clone()).await;
    if outcome.joined {
        audit!(
            state,
            user,
            "inbox.link.join",
            "ConversationInviteLink",
            outcome.link_id,
            serde_json::json!({ "conversation_id": outcome.conversation_id })
        );
    }
    Ok(Json(JoinedConversation {
        conversation: visible_conversation(&state, &Reader::own(&sub), &outcome.conversation_id)
            .await?,
        joined: outcome.joined,
    }))
}

//! A bot invites a person into the group chat it is in, after its owner approved the call. The
//! person is named by their exact username or email address and resolved here, never searched.
//! The invitation is the room's ordinary one with the bot as inviter and the approved reason on
//! it: the person accepts or declines, and blocks between them and the bot's owner apply. Someone
//! away for a week also gets it by email. An address without an account gets a single-use link
//! by email instead, which joins them once they signed up.

use async_trait::async_trait;
use axum::{
    Extension, Json,
    extract::{Path, State},
    http::StatusCode,
};
use flow_like_bot_tools::{InviteOutcome, InviteTarget, RoomInvites, rooms::checked_reason};
use flow_like_types::inbox::{ConversationEncryption, ConversationKind, PrincipalKind};
use sea_orm::{
    ConnectionTrait, DbErr, EntityTrait, QueryFilter, QuerySelect,
    sea_query::{Expr, ExprTrait, Func, SimpleExpr},
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use utoipa::ToSchema;

use super::{room_invite_mail, user_context::backend_error};
use crate::{
    bot_caller::{self, Names},
    entity::{conversation, conversation_member, user},
    error::ApiError,
    middleware::jwt::AppUser,
    routes::{
        inbox::{
            self, INBOX_INVITE_LIMIT, MEMBER_ACTIVE, MEMBER_PENDING,
            invitations::{InviteMode, Invitee, invite_and_notify},
            links::{self, EmailInvite, MAX_OWNER_EMAIL_INVITES_PER_DAY},
            members::Principal,
        },
        user::identity::{public_display_name, public_handle},
    },
    state::AppState,
};

pub(crate) use flow_like_bot_tools::rooms::{
    NO_SUCH_PERSON_CODE as BOT_INVITE_NO_SUCH_PERSON, REFUSED_CODE as BOT_INVITE_REFUSED,
};

#[cfg(test)]
#[path = "rooms_tests.rs"]
mod tests;

const UNNAMED: &str = "This person";

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct RoomInviteRequest {
    /// The person's exact username (a leading @ is fine) or exact email address.
    pub person: String,
    /// Why they should join, as you approved it, at most 500 characters on one line. The person
    /// reads it with the invitation and in any email. Without it, an address that has no account
    /// is not invited.
    #[serde(default)]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RoomInviteResponse {
    /// `invited`, `emailed` (no account had the address, so an email invitation went out),
    /// `already_member` or `already_invited`.
    pub status: String,
    /// The person's public name, never their email address; for an invitation by email, the
    /// address you gave.
    pub name: String,
}

/// Every account whose username or email is exactly `target`, ignoring case, at most two: a
/// second one makes the address ambiguous. A username only matches one other people may see.
async fn accounts_named<C: ConnectionTrait>(
    db: &C,
    target: &InviteTarget,
) -> Result<Vec<(String, Option<String>, Option<String>)>, DbErr> {
    let (column, value) = match target {
        InviteTarget::Username(handle) if public_handle(handle).is_some() => {
            (user::Column::PreferredUsername, handle)
        }
        InviteTarget::Username(_) => return Ok(Vec::new()),
        InviteTarget::Email(address) => (user::Column::Email, address),
    };
    user::Entity::find()
        .select_only()
        .column(user::Column::Id)
        .column(user::Column::Name)
        .column(user::Column::PreferredUsername)
        .filter(
            SimpleExpr::from(Expr::expr(Func::lower(Expr::col(column)))).eq(value.to_lowercase()),
        )
        .limit(2)
        .into_tuple()
        .all(db)
        .await
}

/// Whether the person is already in the room, or holds an unanswered invitation to it.
#[derive(Debug, Clone, Copy)]
enum Presence {
    Member,
    Invited,
}

impl Presence {
    fn outcome(self, name: String) -> InviteOutcome {
        match self {
            Self::Member => InviteOutcome::AlreadyMember { name },
            Self::Invited => InviteOutcome::AlreadyInvited { name },
        }
    }
}

async fn presence<C: ConnectionTrait>(
    db: &C,
    conversation_id: &str,
    person_id: &str,
) -> Result<Option<Presence>, DbErr> {
    let row = conversation_member::Entity::find_by_id((
        conversation_id.to_string(),
        person_id.to_string(),
    ))
    .one(db)
    .await?;
    Ok(row.and_then(|member| {
        if member.status == MEMBER_PENDING {
            Some(Presence::Invited)
        } else if member.status == MEMBER_ACTIVE && member.left_at.is_none() {
            Some(Presence::Member)
        } else {
            None
        }
    }))
}

/// The bot must be an active member of a plain group. A direct conversation takes no
/// invitations, and an encrypted room none from a bot: a member without inbox keys would stop
/// every bot turn there, the reason encrypted rooms have no invite links either.
async fn bot_group<C: ConnectionTrait>(
    db: &C,
    bot_id: &str,
    conversation_id: &str,
) -> Result<(), ApiError> {
    bot_caller::live_owner(db, bot_id).await?;
    inbox::active_membership(db, conversation_id, bot_id).await?;
    let (kind, encryption) = conversation::Entity::find_by_id(conversation_id)
        .select_only()
        .column(conversation::Column::Kind)
        .column(conversation::Column::Encryption)
        .into_tuple::<(String, String)>()
        .one(db)
        .await?
        .unwrap_or_default();
    let refused = |why: &str| {
        Err(ApiError::coded(
            StatusCode::CONFLICT,
            BOT_INVITE_REFUSED,
            why,
        ))
    };
    if ConversationKind::parse(&kind) != Some(ConversationKind::Group) {
        return refused("only group chats take invitations; this is a direct conversation");
    }
    if ConversationEncryption::parse(&encryption) != Some(ConversationEncryption::Plain) {
        return refused(
            "this room is end-to-end encrypted, so a member has to invite people from its member list",
        );
    }
    Ok(())
}

/// What an invitation call does before anything is written.
#[derive(Debug)]
pub(crate) enum InvitePlan {
    /// Nothing to write: the answer is known.
    Settled(InviteOutcome),
    Invite {
        invitee: Invitee,
        name: String,
    },
}

/// Checks that `bot_id` is in the group, resolves `target` exactly and skips a person who is
/// already in the room or invited.
pub(crate) async fn plan_invite<C: ConnectionTrait>(
    db: &C,
    bot_id: &str,
    conversation_id: &str,
    target: &InviteTarget,
) -> Result<InvitePlan, ApiError> {
    bot_group(db, bot_id, conversation_id).await?;
    let mut accounts = accounts_named(db, target).await?;
    if accounts.len() > 1 {
        return Ok(InvitePlan::Settled(InviteOutcome::NotInvitable {
            why: "more than one account uses this address; ask for their username".into(),
        }));
    }
    let Some((person_id, name, handle)) = accounts.pop() else {
        return Ok(InvitePlan::Settled(InviteOutcome::NoSuchPerson));
    };
    let name = public_display_name(name.as_deref(), handle.as_deref(), None)
        .unwrap_or_else(|| UNNAMED.to_string());
    if let Some(presence) = presence(db, conversation_id, &person_id).await? {
        return Ok(InvitePlan::Settled(presence.outcome(name)));
    }
    Ok(InvitePlan::Invite {
        invitee: Invitee {
            principal: Principal {
                id: person_id,
                kind: PrincipalKind::User,
            },
            mode: InviteMode::Invite,
            owner_id: None,
            reason: None,
        },
        name,
    })
}

/// The answer after the invitation was written: a skipped person is in the room or invited by
/// now, or blocked.
pub(crate) async fn settle_invite<C: ConnectionTrait>(
    db: &C,
    conversation_id: &str,
    person_id: &str,
    name: String,
    invited: bool,
) -> Result<InviteOutcome, DbErr> {
    if invited {
        return Ok(InviteOutcome::Invited { name });
    }
    Ok(match presence(db, conversation_id, person_id).await? {
        Some(presence) => presence.outcome(name),
        None => InviteOutcome::NotInvitable {
            why: "they don't take invitations from this bot".into(),
        },
    })
}

/// Sends `bot_id`'s single-use link into the group to an address no account has. An email that
/// could not be sent takes its link back, so asking again tries again.
async fn invite_by_email(
    state: &AppState,
    bot_id: &str,
    conversation_id: &str,
    address: &str,
    reason: &str,
) -> Result<InviteOutcome, ApiError> {
    let Some(mail) = state.mail_client.clone() else {
        return Ok(InviteOutcome::NotInvitable {
            why: "no account uses this address, and this hub sends no email to invite them; ask for their username once they have an account".into(),
        });
    };
    let owner_id = bot_caller::live_owner(&state.db, bot_id).await?;
    let invited = links::email_invite(state, conversation_id, bot_id, &owner_id, address, reason);
    let link = match invited.await? {
        EmailInvite::Created(link) => link,
        EmailInvite::Pending => {
            return Ok(InviteOutcome::AlreadyInvited {
                name: address.to_string(),
            });
        }
        EmailInvite::DailyLimit => {
            return Ok(InviteOutcome::NotInvitable {
                why: format!(
                    "your owner's bots already sent {MAX_OWNER_EMAIL_INVITES_PER_DAY} email invitations in the last day"
                ),
            });
        }
    };
    if let Err(error) = room_invite_mail::send_to_newcomer(state, &mail, &link).await {
        if let Err(cleanup) = links::drop_email_invite(&state.db, &link.id).await {
            tracing::warn!(%cleanup, link_id = %link.id, "Failed to withdraw an email invitation that was not sent");
        }
        return Err(error);
    }
    tracing::info!(link_id = %link.id, conversation_id, bot_id, "A bot invited an address by email");
    Ok(InviteOutcome::Emailed)
}

/// Invites the person `target` names into `conversation_id` as `bot_id`, which must be in it,
/// with the `reason` its owner approved. An address without an account is invited by email when
/// there is a reason to send; a person away for a week gets an email beside the invitation.
pub(crate) async fn invite_into_room(
    state: &AppState,
    bot_id: &str,
    conversation_id: &str,
    target: &InviteTarget,
    reason: Option<&str>,
) -> Result<InviteOutcome, ApiError> {
    let (mut invitee, name) = match plan_invite(&state.db, bot_id, conversation_id, target).await? {
        InvitePlan::Settled(InviteOutcome::NoSuchPerson) => {
            return match (target, reason) {
                (InviteTarget::Email(address), Some(reason)) => {
                    invite_by_email(state, bot_id, conversation_id, address, reason).await
                }
                _ => Ok(InviteOutcome::NoSuchPerson),
            };
        }
        InvitePlan::Settled(outcome) => return Ok(outcome),
        InvitePlan::Invite { invitee, name } => (invitee, name),
    };
    invitee.reason = reason.map(str::to_string);
    let person_id = invitee.principal.id.clone();
    let outcome = match invite_and_notify(
        state,
        bot_id,
        conversation_id,
        vec![invitee],
        BTreeMap::new(),
    )
    .await
    {
        Ok(outcome) => outcome,
        Err(error) if error.public_code() == INBOX_INVITE_LIMIT => {
            return Ok(InviteOutcome::NotInvitable {
                why: "this room already holds the most pending invitations it can".into(),
            });
        }
        Err(error) => return Err(error),
    };
    let invited = !outcome.invited.is_empty();
    if invited {
        room_invite_mail::email_if_away(state, bot_id, conversation_id, &person_id, reason).await;
    }
    Ok(settle_invite(&state.db, conversation_id, &person_id, name, invited).await?)
}

/// `room_invite` for one bot in the conversation its server turn runs in.
pub(crate) struct ServerRoomInvites {
    state: AppState,
    bot_id: String,
    conversation_id: String,
}

impl ServerRoomInvites {
    pub(crate) fn new(state: &AppState, bot_id: &str, conversation_id: &str) -> Self {
        Self {
            state: state.clone(),
            bot_id: bot_id.to_owned(),
            conversation_id: conversation_id.to_owned(),
        }
    }
}

#[async_trait]
impl RoomInvites for ServerRoomInvites {
    async fn invite(
        &self,
        person: &InviteTarget,
        reason: &str,
    ) -> flow_like_types::Result<InviteOutcome> {
        invite_into_room(
            &self.state,
            &self.bot_id,
            &self.conversation_id,
            person,
            Some(reason),
        )
        .await
        .map_err(backend_error)
    }
}

/// The bot's owner, or the bot's token for a turn in exactly this room.
async fn authorized(
    state: &AppState,
    user: &AppUser,
    bot_id: &str,
    conversation_id: &str,
) -> Result<(), ApiError> {
    if let AppUser::Bot(token) = user
        && token.conv_id.is_none()
    {
        return Err(ApiError::forbidden(
            "A bot invites people only from a turn in that group chat",
        ));
    }
    let owner = bot_caller::acting_owner(
        &state.db,
        user,
        &Names {
            as_bot: Some(bot_id),
            conversation: Some(conversation_id),
            turn: None,
        },
    )
    .await?;
    super::owned_bot(&state.db, &owner, bot_id).await?;
    Ok(())
}

fn response_of(
    outcome: InviteOutcome,
    person: &InviteTarget,
) -> Result<RoomInviteResponse, ApiError> {
    let (status, name) = match outcome {
        InviteOutcome::Invited { name } => ("invited", name),
        InviteOutcome::Emailed => ("emailed", person.value().to_string()),
        InviteOutcome::AlreadyMember { name } => ("already_member", name),
        InviteOutcome::AlreadyInvited { name } => ("already_invited", name),
        InviteOutcome::NoSuchPerson => {
            return Err(ApiError::coded(
                StatusCode::NOT_FOUND,
                BOT_INVITE_NO_SUCH_PERSON,
                format!(
                    "No account has the {} '{}'",
                    if person.is_email() {
                        "email address"
                    } else {
                        "username"
                    },
                    person.value()
                ),
            ));
        }
        InviteOutcome::NotInvitable { why } => {
            return Err(ApiError::coded(
                StatusCode::CONFLICT,
                BOT_INVITE_REFUSED,
                why,
            ));
        }
    };
    Ok(RoomInviteResponse {
        status: status.into(),
        name,
    })
}

#[utoipa::path(
    post,
    path = "/bots/{bot_id}/rooms/{conversation_id}/invitations",
    tag = "Bots",
    description = "Invite a person into a group chat as one of your bots, after you approved it. Name the person by their exact username or email address; nobody is searched. The person reads your reason with the invitation and joins once they accept. Someone who has not opened Flow-Like for a week also gets it by email. An address without an account gets an email with a link to sign up and join, which works once for 30 days.",
    params(
        ("bot_id" = String, Path, description = "Your bot, a member of the group chat"),
        ("conversation_id" = String, Path, description = "The group chat")
    ),
    request_body = RoomInviteRequest,
    responses(
        (status = 200, description = "Invited or emailed, or already in the room or invited", body = RoomInviteResponse),
        (status = 400, description = "Not one username or email address, or a reason that is empty, longer than 500 characters or more than one line"),
        (status = 401, description = "Not signed in"),
        (status = 403, description = "A bot token from another conversation"),
        (status = 404, description = "BOT_INVITE_NO_SUCH_PERSON: no account has this username, or this address without a reason to email; or not your bot, or the bot is not in this conversation"),
        (status = 409, description = "BOT_INVITE_REFUSED: a direct conversation, a full invitation list, an ambiguous address, a person who takes no invitations from this bot, your bots having sent their 20 email invitations of the day, or a hub that sends no email"),
        (status = 500, description = "The email invitation could not be sent; nothing was kept, so you can ask again")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(
    name = "POST /bots/{bot_id}/rooms/{conversation_id}/invitations",
    skip_all
)]
pub async fn invite_person(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((bot_id, conversation_id)): Path<(String, String)>,
    Json(body): Json<RoomInviteRequest>,
) -> Result<Json<RoomInviteResponse>, ApiError> {
    authorized(&state, &user, &bot_id, &conversation_id).await?;
    let person = InviteTarget::parse(&body.person).ok_or_else(|| {
        ApiError::bad_request("'person' must be one exact username or one email address")
    })?;
    let reason = body
        .reason
        .as_deref()
        .map(|reason| {
            checked_reason(reason).ok_or_else(|| {
                ApiError::bad_request(format!(
                    "'reason' is one line of 1 to {} characters",
                    flow_like_types::bot_message::MAX_SUMMARY_CHARS
                ))
            })
        })
        .transpose()?;
    let outcome = invite_into_room(
        &state,
        &bot_id,
        &conversation_id,
        &person,
        reason.as_deref(),
    )
    .await?;
    response_of(outcome, &person).map(Json)
}

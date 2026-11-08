//! A bot as its own principal. The token names the bot, its turn and (in a room) the
//! conversation; the owner's live role, the bot's restrictions for the app and the room's limits
//! are read from the database on every call, so a revoked grant, a left room, a deleted bot or
//! an owner who lost the app takes effect on the next request.

use crate::{
    backend_jwt::{self, BackendJwtError, TokenType, issuer, make_time_claims},
    entity::{app, bot, bot_app, conversation_member, role},
    error::ApiError,
    middleware::jwt::{AppPermissionResponse, BotUser, member_role},
    permission::role_permission::RolePermissions,
    state::AppState,
};
use flow_like::flow::copilot::EffectivePermissions;
use flow_like::flow::execution::ExecutionPrincipal;
use flow_like_types::bot_effective::BotRoomLimits;
use flow_like_types::bot_restrictions::BotScopes;
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QuerySelect};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// A turn token must cover the host deadline and the final reply. Direct tokens retain the
/// shorter `TokenType::Bot` default.
pub const MAX_BOT_TOKEN_TTL_SECONDS: i64 =
    flow_like_types::bot_turn::DEFAULT_TURN_DEADLINE_SECS as i64 + 120;

const PRINCIPAL_KIND_BOT: &str = "BOT";

/// Claims of a bot token. There is deliberately no `sub`, no owner and no mask.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BotClaims {
    pub bot_id: String,
    pub turn_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conv_id: Option<String>,
    #[serde(rename = "typ")]
    pub token_type: TokenType,
    pub iss: String,
    pub aud: String,
    pub iat: i64,
    pub nbf: i64,
    pub exp: i64,
    pub jti: String,
}

pub struct SignedBotToken {
    pub token: String,
    pub expires_at: i64,
}

pub fn sign_bot_token(
    user: &BotUser,
    ttl_seconds: Option<i64>,
) -> Result<SignedBotToken, BackendJwtError> {
    let ttl = ttl_seconds
        .unwrap_or_else(|| TokenType::Bot.default_ttl_seconds())
        .clamp(60, MAX_BOT_TOKEN_TTL_SECONDS);
    let time = make_time_claims(TokenType::Bot, Some(ttl));
    let claims = BotClaims {
        bot_id: user.bot_id.clone(),
        turn_id: user.turn_id.clone(),
        conv_id: user.conv_id.clone(),
        token_type: TokenType::Bot,
        iss: issuer().to_string(),
        aud: TokenType::Bot.audience().to_string(),
        iat: time.iat,
        nbf: time.nbf,
        exp: time.exp,
        jti: flow_like_types::create_id(),
    };
    Ok(SignedBotToken {
        token: backend_jwt::sign(&claims)?,
        expires_at: claims.exp,
    })
}

pub fn verify_bot_token(token: &str) -> Result<BotClaims, BackendJwtError> {
    let claims: BotClaims = backend_jwt::verify(token, TokenType::Bot)?;
    if claims.token_type != TokenType::Bot || claims.bot_id.is_empty() || claims.turn_id.is_empty()
    {
        return Err(BackendJwtError::DecodingError(
            "Invalid bot token".to_string(),
        ));
    }
    Ok(claims)
}

/// What a bot holds in one app right now.
#[derive(Debug, Clone)]
pub struct BotGrant {
    pub bot_id: String,
    pub owner_id: String,
    pub role: role::Model,
    pub permissions: RolePermissions,
    pub effective: EffectivePermissions,
}

fn refusal(reason: &'static str) -> ApiError {
    ApiError::forbidden(reason)
}

async fn live_bot<C: ConnectionTrait>(db: &C, bot_id: &str) -> Result<bot::Model, ApiError> {
    bot::Entity::find_by_id(bot_id)
        .filter(bot::Column::DeletedAt.is_null())
        .one(db)
        .await?
        .ok_or_else(|| ApiError::unauthorized("Bot is no longer available"))
}

async fn is_workspace<C: ConnectionTrait>(
    db: &C,
    bot_id: &str,
    app_id: &str,
) -> Result<bool, ApiError> {
    Ok(app::Entity::find_by_id(app_id)
        .select_only()
        .column(app::Column::Id)
        .filter(app::Column::BotId.eq(bot_id))
        .into_tuple::<String>()
        .one(db)
        .await?
        .is_some())
}

/// The bot's row restrictions in `app_id`: `None` when the app is neither attached nor the
/// bot's workspace, which leaves the bot without access.
async fn app_restrictions<C: ConnectionTrait>(
    db: &C,
    bot_id: &str,
    app_id: &str,
) -> Result<Option<(Option<i64>, BotScopes)>, ApiError> {
    if let Some(row) = bot_app::Entity::find_by_id((bot_id.to_string(), app_id.to_string()))
        .one(db)
        .await?
    {
        let scopes = serde_json::from_value::<BotScopes>(row.scopes.clone()).map_err(|error| {
            tracing::error!(%error, bot_id, app_id, "Bot app scopes are unreadable");
            refusal("Bot restrictions for this app are unreadable")
        })?;
        return Ok(Some((row.permission_mask, scopes)));
    }
    Ok(is_workspace(db, bot_id, app_id)
        .await?
        .then(|| (None, BotScopes::default())))
}

/// The limits its owner set when accepting the bot's invitation into `conv_id`; the bot must be an active member.
pub(crate) async fn room_limits<C: ConnectionTrait>(
    db: &C,
    bot_id: &str,
    conv_id: &str,
) -> Result<BotRoomLimits, ApiError> {
    let member = conversation_member::Entity::find()
        .filter(conversation_member::Column::ConversationId.eq(conv_id))
        .filter(conversation_member::Column::PrincipalId.eq(bot_id))
        .filter(conversation_member::Column::PrincipalKind.eq(PRINCIPAL_KIND_BOT))
        .filter(crate::routes::inbox::active_member())
        .one(db)
        .await?
        .ok_or_else(|| refusal("Bot is not a member of this room"))?;
    match member.limits {
        None => Ok(BotRoomLimits::default()),
        Some(limits) => serde_json::from_value(limits).map_err(|error| {
            tracing::error!(%error, bot_id, conv_id, "Bot room limits are unreadable");
            refusal("Bot limits for this room are unreadable")
        }),
    }
}

/// bot live and attached → owner's live role → expand → strip `BOT_NEVER` → bot row mask → room
/// limits → resource scopes. Any missing link answers with a refusal.
pub async fn bot_grant_in<C: ConnectionTrait>(
    db: &C,
    bot: &BotUser,
    app_id: &str,
) -> Result<BotGrant, ApiError> {
    let row = live_bot(db, &bot.bot_id).await?;
    let Some((mask, scopes)) = app_restrictions(db, &bot.bot_id, app_id).await? else {
        return Err(refusal("Bot is not attached to this app"));
    };
    let owner_role = member_role(db, &row.owner_id, app_id)
        .await?
        .ok_or_else(|| refusal("The bot's owner has no access to this app"))?;
    let room = match bot.conv_id.as_deref() {
        Some(conv_id) => Some(room_limits(db, &bot.bot_id, conv_id).await?),
        None => None,
    };
    let effective =
        EffectivePermissions::resolve(owner_role.permissions, app_id, mask, &scopes, room.as_ref());
    let bits = effective.role_mask.unwrap_or_default();
    let permissions = RolePermissions::from_bits_truncate(bits);
    if permissions.is_empty() {
        return Err(refusal("Bot holds no permission in this app"));
    }
    Ok(BotGrant {
        bot_id: row.id,
        owner_id: row.owner_id,
        role: role::Model {
            permissions: permissions.bits(),
            ..owner_role
        },
        permissions,
        effective,
    })
}

/// The single resolution point `AppUser::app_permission` calls for a bot token.
pub async fn bot_app_permission(
    bot: &BotUser,
    app_id: &str,
    state: &AppState,
) -> Result<AppPermissionResponse, ApiError> {
    Ok(bot_permission_with_grant(bot, app_id, state).await?.0)
}

/// [`bot_app_permission`] together with the grant it was built from.
pub async fn bot_permission_with_grant(
    bot: &BotUser,
    app_id: &str,
    state: &AppState,
) -> Result<(AppPermissionResponse, BotGrant), ApiError> {
    let grant = bot_grant_in(&state.db, bot, app_id).await?;
    let response = AppPermissionResponse {
        state: state.clone(),
        permissions: grant.permissions,
        role: Arc::new(grant.role.clone()),
        sub: None,
        effective_user_id: Some(grant.owner_id.clone()),
        technical_user_id: Some(grant.bot_id.clone()),
        identifier: grant.bot_id.clone(),
        principal: ExecutionPrincipal::Bot,
        origin_app_id: None,
    };
    Ok((response, grant))
}

#[cfg(test)]
#[path = "bot_permission_tests.rs"]
mod tests;

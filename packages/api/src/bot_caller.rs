//! Rules for routes that a bot token may call. A bot acts only as itself: `?as=` must be the
//! token's bot, a turn route names the token's turn, a room route the token's room. The owner is
//! never read from the token; it is resolved from the live bot row.

use crate::{entity::bot, error::ApiError, middleware::jwt::{AppUser, BotUser}};
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter};

/// What a route names that a bot token must match.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct Names<'a> {
    pub as_bot: Option<&'a str>,
    pub conversation: Option<&'a str>,
    pub turn: Option<&'a str>,
}

fn trimmed(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

pub(crate) fn ensure_as_self(bot: &BotUser, as_bot: Option<&str>) -> Result<(), ApiError> {
    match trimmed(as_bot) {
        Some(named) if named != bot.bot_id => Err(ApiError::forbidden(
            "A bot token acts only as its own bot",
        )),
        _ => Ok(()),
    }
}

pub(crate) fn ensure_turn(bot: &BotUser, turn_id: &str) -> Result<(), ApiError> {
    if bot.turn_id == turn_id {
        Ok(())
    } else {
        Err(ApiError::forbidden("This bot token belongs to another turn"))
    }
}

pub(crate) fn ensure_room(bot: &BotUser, conversation_id: &str) -> Result<(), ApiError> {
    match bot.conv_id.as_deref() {
        Some(room) if !room.eq_ignore_ascii_case(conversation_id) => Err(ApiError::forbidden(
            "This bot token belongs to another conversation",
        )),
        _ => Ok(()),
    }
}

pub(crate) fn check(bot: &BotUser, names: &Names<'_>) -> Result<(), ApiError> {
    ensure_as_self(bot, names.as_bot)?;
    if let Some(conversation) = names.conversation {
        ensure_room(bot, conversation)?;
    }
    if let Some(turn) = names.turn {
        ensure_turn(bot, turn)?;
    }
    Ok(())
}

pub(crate) async fn live_owner<C: ConnectionTrait>(
    db: &C,
    bot_id: &str,
) -> Result<String, ApiError> {
    bot::Entity::find_by_id(bot_id)
        .filter(bot::Column::DeletedAt.is_null())
        .one(db)
        .await?
        .map(|row| row.owner_id)
        .ok_or_else(|| ApiError::unauthorized("Bot is no longer available"))
}

/// The person on whose behalf a route runs: the caller themselves, or the owner of the bot the
/// token names after `names` matched the token.
pub(crate) async fn acting_owner<C: ConnectionTrait>(
    db: &C,
    user: &AppUser,
    names: &Names<'_>,
) -> Result<String, ApiError> {
    match user {
        AppUser::Bot(bot) => {
            check(bot, names)?;
            live_owner(db, &bot.bot_id).await
        }
        _ => user.sub(),
    }
}

/// The bot a write names: the token's own bot, or `as` for a person.
pub(crate) fn bot_named<'a>(user: &'a AppUser, as_bot: Option<&'a str>) -> Option<&'a str> {
    match user {
        AppUser::Bot(bot) => Some(bot.bot_id.as_str()),
        _ => trimmed(as_bot),
    }
}

#[cfg(test)]
#[path = "bot_caller_tests.rs"]
mod tests;
